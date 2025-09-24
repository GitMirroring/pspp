// PSPP - a program for statistical analysis.
// Copyright (C) 2025 Free Software Foundation, Inc.
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE.  See the GNU General Public License for more
// details.
//
// You should have received a copy of the GNU General Public License along with
// this program.  If not, see <http://www.gnu.org/licenses/>.

//! Data cell contents.
//!
//! This module contains [Value], which is the contents of a single pivot table
//! cell, plus what it in turn contains.

// Warn about missing docs, but not for items declared with `#[cfg(test)]`.
#![cfg_attr(not(test), warn(missing_docs))]
#![warn(dead_code)]

use crate::{
    calendar::{date_time_to_pspp, time_to_pspp},
    data::{ByteString, Datum, EncodedString, WithEncoding},
    format::{self, DATETIME40_0, Decimal, F8_2, F40, Format, TIME40_0, Type, UncheckedFormat},
    output::pivot::{
        Footnote, FootnoteMarkerType,
        look::{CellStyle, FontStyle},
    },
    settings::{Settings, Show},
    spv::html::Markup,
    variable::{VarType, Variable},
};
use chrono::{NaiveDateTime, NaiveTime};
use itertools::Itertools;
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeStruct},
};
use std::{
    borrow::Borrow,
    fmt::{Debug, Display, Write},
    iter::{once, repeat},
    sync::Arc,
};

/// The content of a single pivot table cell.
///
/// A [Value] is also a pivot table's title, caption, footnote marker and
/// contents, and so on.
#[derive(Clone, Default, PartialEq)]
pub struct Value {
    /// Content.
    pub inner: ValueInner,

    /// Optional styling.
    pub styling: Option<Box<ValueStyle>>,
}

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}
/// Wrapper for [Value] that serializes in a plain way:
///
/// - Numbers: The number.
/// - Strings: The string.
/// - Variables: The variable name.
/// - Text: The localized text string.
/// - Markup: A string containing HTML for the markup.
/// - Template: The formatted template string.
/// - Empty: `()`.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BareValue<T>(pub T);
impl<T> Serialize for BareValue<T>
where
    T: Borrow<Value>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let value = self.0.borrow();
        match &value.inner {
            ValueInner::Datum(datum_value) => datum_value.serialize_bare(serializer),
            ValueInner::Variable(variable_value) => variable_value.var_name.serialize(serializer),
            ValueInner::Text(text_value) => text_value.localized.serialize(serializer),
            ValueInner::Markup(markup) => markup.serialize(serializer),
            ValueInner::Template(_) => value.display(()).to_string().serialize(serializer),
            ValueInner::Empty => ().serialize(serializer),
        }
    }
}
impl Value {
    /// Constructs a new `Value`, initially with no styling.
    ///
    /// Usually one of the other constructors is more convenient.
    pub fn new(inner: ValueInner) -> Self {
        Self {
            inner,
            styling: None,
        }
    }

    /// Constructs a new `Value` from `number` with a default [F8_2] format.
    /// Some related useful methods are:
    ///
    /// - [with_source_variable], to add information about the variable that the
    ///   datum came from (or use [new_datum_from_variable] as a shortcut to
    ///   combine both).
    ///
    /// - [with_format] to override the default format.
    ///
    /// [with_source_variable]: Self::with_source_variable
    /// [new_datum_from_variable]: Self::new_datum_from_variable
    /// [with_format]: Self::with_format
    pub fn new_number(number: Option<f64>) -> Self {
        Self::new(ValueInner::Datum(DatumValue::new_number(number)))
    }

    /// Construct a new `Value` from `number` with format [F40].
    pub fn new_integer(x: Option<f64>) -> Self {
        Self::new_number(x).with_format(F40)
    }

    /// Constructs a new `Value` as a number whose value is `date_time`, which
    /// is converted to the [PSPP date representation](crate::calendar), with
    /// format [DATETIME40_0].
    pub fn new_date(date_time: NaiveDateTime) -> Self {
        Self::new_number(Some(date_time_to_pspp(date_time))).with_format(DATETIME40_0)
    }

    /// Constructs a new `Value` as a number whose value is `time`, which is
    /// converted to the [PSPP time representation](crate::calendar), with
    /// format [TIME40_0].
    pub fn new_time(time: NaiveTime) -> Self {
        Self::new_number(Some(time_to_pspp(time))).with_format(TIME40_0)
    }

    /// Constructs a new `Value` from localizable text string `s`.
    ///
    /// PSPP doesn't support internationalization yet, so this does the same
    /// thing as [new_user_text] for now.
    ///
    /// [new_user_text]: Self::new_user_text
    pub fn new_text(s: impl Into<String>) -> Self {
        Self::new_user_text(s)
    }

    /// Constructs a new `Value` from localizable text string `localized`,
    /// English string `c`, and identifier `id`.  If the string came from the
    /// user, `user_provided` should be true.
    pub fn new_general_text(localized: String, c: String, id: String, user_provided: bool) -> Self {
        Self::new(ValueInner::Text(TextValue {
            user_provided,
            c: (c != localized).then_some(c),
            id: (id != localized).then_some(id),
            localized,
        }))
    }

    /// Constructs a new `Value` from `markup`.
    pub fn new_markup(markup: Markup) -> Self {
        Self::new(ValueInner::Markup(markup))
    }

    /// Constructs a new text `Value` from `s`, which should have been provided
    /// by the user.
    pub fn new_user_text(s: impl Into<String>) -> Self {
        Self::new(ValueInner::new_user_text(s))
    }

    /// Constructs a new `Value` from `variable`.
    pub fn new_variable(variable: &Variable) -> Self {
        Self::new(ValueInner::Variable(VariableValue {
            show: None,
            var_name: String::from(variable.name.as_str()),
            variable_label: variable.label.clone(),
        }))
    }

    /// Constructs a new `Value` from `datum` with a default format.  Some
    /// related useful methods are:
    ///
    /// - [with_source_variable], to add information about the variable that the
    ///   datum came from (or use [new_datum_from_variable] as a shortcut to
    ///   combine both).
    ///
    /// - [with_format] to override the default format.
    ///
    /// [with_source_variable]: Self::with_source_variable
    /// [new_datum_from_variable]: Self::new_datum_from_variable
    /// [with_format]: Self::with_format
    pub fn new_datum<B>(datum: &Datum<B>) -> Self
    where
        B: EncodedString,
    {
        Self::new(ValueInner::Datum(DatumValue::new(datum)))
    }

    /// Construct a new, empty `Value`.
    pub const fn new_empty() -> Self {
        // Can't use `Self::default()` because that is non-const.
        Value {
            inner: ValueInner::Empty,
            styling: None,
        }
    }

    /// Returns a reference to a statically allocated empty `Value`.
    pub const fn static_empty() -> &'static Self {
        static EMPTY: Value = Value::new_empty();
        &EMPTY
    }

    /// Returns true if this `Value` is empty and unstyled.
    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty() && self.styling.is_none()
    }

    /// Returns this value with its value label, format, and variable name from
    /// `variable`.
    pub fn with_source_variable(self, variable: &Variable) -> Self {
        let value_label = self
            .datum()
            .and_then(|datum| variable.value_labels.get(&datum).map(String::from));
        self.with_value_label(value_label)
            .with_format(variable.print_format)
            .with_variable_name(Some(variable.name.as_str().into()))
    }

    /// Returns this value with its display format set to `format`, if it is a
    /// [DatumValue].
    pub fn with_format(self, format: impl Into<ValueFormat>) -> Self {
        Self {
            inner: self.inner.with_format(format),
            ..self
        }
    }

    /// Construct a new `Value` from `datum`, which is a value of `variable`.
    pub fn new_datum_from_variable(datum: &Datum<ByteString>, variable: &Variable) -> Self {
        Self::new_datum(&datum.as_encoded(variable.encoding())).with_source_variable(variable)
    }

    /// Returns the inner [Datum], if this value is a [DatumValue].
    pub fn datum(&self) -> Option<&Datum<WithEncoding<ByteString>>> {
        self.inner.datum()
    }

    /// Returns this `Value` with the added `footnote`.
    pub fn with_footnote(mut self, footnote: &Arc<Footnote>) -> Self {
        self.add_footnote(footnote);
        self
    }

    /// Returns this `Value` with the added `footnotes`.
    pub fn with_footnotes<'a>(
        mut self,
        footnotes: impl IntoIterator<Item = &'a Arc<Footnote>>,
    ) -> Self {
        for footnote in footnotes {
            self.add_footnote(footnote);
        }
        self
    }

    /// Adds `footnote` to this `Value`.
    pub fn add_footnote(&mut self, footnote: &Arc<Footnote>) {
        let footnotes = &mut self.styling_mut().footnotes;
        footnotes.push(footnote.clone());
        footnotes.sort_by_key(|f| f.index);
    }

    /// Removes all of the footnotes from `value`.
    pub fn clear_footnotes(&mut self) {
        if let Some(styling) = &mut self.styling
            && !styling.footnotes.is_empty()
        {
            styling.footnotes.clear();
            if styling.is_empty() {
                self.styling = None;
            }
        }
    }

    /// Returns this `Value` with the added `subscripts`.
    pub fn with_subscripts<'a>(
        mut self,
        subscripts: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.styling_mut()
            .subscripts
            .extend(subscripts.into_iter().map(|s| s.into()));
        self
    }

    /// Returns this `Value` with the added `subscript`.
    pub fn with_subscript(mut self, subscript: impl Into<String>) -> Self {
        self.add_subscript(subscript);
        self
    }

    /// Adds `subscript` to this `Value`.
    pub fn add_subscript(&mut self, subscript: impl Into<String>) {
        self.styling_mut().subscripts.push(subscript.into());
    }

    /// Returns this `Value` with `show` as the [Show] setting for value labels,
    /// if this is a [DatumValue].
    pub fn with_show_value_label(mut self, show: Option<Show>) -> Self {
        if let Some(datum_value) = self.inner.as_datum_value_mut() {
            datum_value.show = show;
        }
        self
    }

    /// Returns this `Value` with `value_label` as the value label, if this is a
    /// [DatumValue].
    ///
    /// Use [with_source_variable], instead, to automatically add a value label
    /// and other information from a source variable.
    ///
    /// [with_source_variable]: Self::with_source_variable
    pub fn with_value_label(mut self, value_label: Option<String>) -> Self {
        if let Some(datum_value) = self.inner.as_datum_value_mut() {
            datum_value.value_label = value_label.clone()
        }
        self
    }

    /// Returns this `Value` with `variable_name` as the variable's name, if
    /// this is a [DatumValue].
    ///
    /// Use [with_source_variable], instead, to automatically add a variable
    /// name and other information from a source variable.
    ///
    /// [with_source_variable]: Self::with_source_variable
    pub fn with_variable_name(mut self, variable_name: Option<String>) -> Self {
        if let Some(datum_value) = self.inner.as_datum_value_mut() {
            datum_value.variable = variable_name.clone()
        }
        self
    }

    /// Returns this `Value` with `show` as the [Show] setting for variable
    /// labels, if this is a [VariableValue].
    pub fn with_show_variable_label(mut self, show: Option<Show>) -> Self {
        if let ValueInner::Variable(variable_value) = &mut self.inner {
            variable_value.show = show;
        }
        self
    }

    /// Returns this `Value` with the specified `font_style`.
    pub fn with_font_style(mut self, font_style: FontStyle) -> Self {
        self.set_font_style(font_style);
        self
    }

    /// Sets the `Value`'s font style.
    pub fn set_font_style(&mut self, font_style: FontStyle) {
        self.styling_mut().font_style = Some(font_style);
    }

    /// Returns this `Value` with the specified `cell_style`.
    pub fn with_cell_style(mut self, cell_style: CellStyle) -> Self {
        self.set_cell_style(cell_style);
        self
    }

    /// Sets the `Value`'s cell style.
    pub fn set_cell_style(&mut self, cell_style: CellStyle) {
        self.styling_mut().cell_style = Some(cell_style);
    }

    /// Returns this `Value` with the specified `styling`.
    pub fn with_styling(self, styling: Option<Box<ValueStyle>>) -> Self {
        Self { styling, ..self }
    }

    /// Returns the styling for this `Value` for modification.
    ///
    /// If this `Value` doesn't have styling yet, this creates it.
    pub fn styling_mut(&mut self) -> &mut ValueStyle {
        self.styling.get_or_insert_default()
    }

    /// Returns this `Value`'s font style, if it has one.
    pub fn font_style(&self) -> Option<&FontStyle> {
        self.styling
            .as_ref()
            .map(|styling| styling.font_style.as_ref())
            .flatten()
    }

    /// Returns this `Value`'s cell style, if it has one.
    pub fn cell_style(&self) -> Option<&CellStyle> {
        self.styling
            .as_ref()
            .map(|styling| styling.cell_style.as_ref())
            .flatten()
    }

    /// Returns this `Value`'s subscripts.
    pub fn subscripts(&self) -> &[String] {
        self.styling
            .as_ref()
            .map_or(&[], |styling| &styling.subscripts)
    }

    /// Returns this `Value`'s footnotes.
    pub fn footnotes(&self) -> &[Arc<Footnote>] {
        self.styling
            .as_ref()
            .map_or(&[], |styling| &styling.footnotes)
    }

    /// Returns an object that will format this value, including subscripts and
    /// superscripts and footnotes.  `options` controls whether variable and
    /// value labels are included.
    pub fn display(&self, options: impl Into<ValueOptions>) -> DisplayValue<'_> {
        let display = self.inner.display(options);
        match &self.styling {
            Some(styling) => display.with_styling(styling),
            None => display,
        }
    }

    /// Serializes this value in a plain way, like [BareValue].  This function
    /// can be used on a field as `#[serde(serialize_with =
    /// Value::serialize_bare)]`.
    pub fn serialize_bare<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        BareValue(self).serialize(serializer)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::new_text(value)
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::new_text(value)
    }
}

impl From<&Variable> for Value {
    fn from(variable: &Variable) -> Self {
        Self::new_variable(variable)
    }
}

/// Helper struct for printing a [Value] with `format!` and `{}`.
///
/// Create this struct with [Value::display].
#[derive(Clone, Debug)]
pub struct DisplayValue<'a> {
    inner: &'a ValueInner,
    subscripts: &'a [String],
    footnotes: &'a [Arc<Footnote>],
    options: ValueOptions,
    show_value: bool,
    show_label: Option<&'a str>,
}

impl<'a> DisplayValue<'a> {
    /// Returns the subscripts to be displayed, as an iterator of `&str`.
    pub fn subscripts(&self) -> impl Iterator<Item = &str> + ExactSizeIterator + Clone {
        self.subscripts.iter().map(String::as_str)
    }

    /// Returns true if the value to be displayed includes subscripts.
    pub fn has_subscripts(&self) -> bool {
        !self.subscripts.is_empty()
    }

    /// Returns the footnotes to be displayed, as an iterator.
    ///
    /// The iterator can have fewer elements than there are footnotes, because
    /// footnotes can be hidden.
    pub fn footnotes(&self) -> impl Iterator<Item = impl Display> + Clone {
        self.footnotes
            .iter()
            .filter(|f| f.show)
            .map(|f| f.display_marker(self.options.clone()))
    }

    /// Returns true if there are footnotes to be displayed.
    ///
    /// Because footnotes can be hidden, this method can return false for values
    /// with footnotes.
    pub fn has_footnotes(&self) -> bool {
        self.footnotes().next().is_some()
    }

    /// Returns this [DisplayValue] modified so that it won't show any
    /// subscripts or footnotes.
    pub fn without_suffixes(self) -> Self {
        Self {
            subscripts: &[],
            footnotes: &[],
            ..self
        }
    }

    /// Returns this [DisplayValue] modified so that it will only show the
    /// suffixes and footnotes, not the body.
    pub fn without_body(self) -> Self {
        Self {
            inner: &ValueInner::Empty,
            ..self
        }
    }

    /// Returns the [Markup] to be formatted, if any.
    pub fn markup(&self) -> Option<&Markup> {
        self.inner.as_markup()
    }

    /// Returns this display split into `(body, suffixes)` where `suffixes` is
    /// subscripts and footnotes and `body` is everything else.
    pub fn split(self) -> (Self, Self) {
        (self.clone().without_suffixes(), self.without_body())
    }

    /// Returns this display with subscripts and footnotes taken from `styling`.
    ///
    /// (This display can't use the other parts of `styling`, since we're just
    /// formatting plain text.)
    pub fn with_styling(mut self, styling: &'a ValueStyle) -> Self {
        self.subscripts = styling.subscripts.as_slice();
        self.footnotes = styling.footnotes.as_slice();
        self
    }

    /// Returns this display with the given `subscripts.`
    pub fn with_subscripts(self, subscripts: &'a [String]) -> Self {
        Self { subscripts, ..self }
    }

    /// Returns this display with the given `footnotes.`
    pub fn with_footnotes(self, footnotes: &'a [Arc<Footnote>]) -> Self {
        Self { footnotes, ..self }
    }

    /// Returns true if this display will format to the empty string.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty() && self.subscripts.is_empty() && self.footnotes.is_empty()
    }

    /// Returns the character that the formatted value would use for a decimal
    /// point if it has one, or `None` if the value isn't a datum value and
    /// therefore doesn't have a decimal point.
    pub fn decimal(&self) -> Option<Decimal> {
        self.inner
            .as_datum_value()
            .map(|datum_value| datum_value.decimal())
    }

    fn small(&self) -> f64 {
        self.options.small
    }

    /// Returns a variable type for the value to be displayed.
    ///
    /// We consider a numeric value displayed by itself to be numeric, but if
    /// the value label is displayed then it is considered to be a string.
    /// Anything else is also a string.
    ///
    /// This is useful for passing to [HorzAlign::for_mixed], although maybe
    /// this method should just return [HorzAlign] directly.
    ///
    /// [HorzAlign]: crate::output::pivot::look::HorzAlign
    /// [HorzAlign::for_mixed]: crate::output::pivot::look::HorzAlign::for_mixed
    pub fn var_type(&self) -> VarType {
        if let Some(datum_value) = self.inner.as_datum_value()
            && datum_value.datum.is_number()
            && self.show_label.is_none()
        {
            VarType::Numeric
        } else {
            VarType::String
        }
    }
}

impl Display for DisplayValue<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner {
            ValueInner::Datum(datum_value) => datum_value.display(self, f),
            ValueInner::Variable(variable_value) => variable_value.display(self, f),
            ValueInner::Markup(markup) => write!(f, "{markup}"),
            ValueInner::Text(text_value) => write!(f, "{text_value}"),
            ValueInner::Template(template_value) => template_value.display(self, f),
            ValueInner::Empty => Ok(()),
        }?;

        for (subscript, delimiter) in self.subscripts.iter().zip(once('_').chain(repeat(','))) {
            write!(f, "{delimiter}{subscript}")?;
        }

        if !self.footnotes.is_empty() {
            write!(
                f,
                "[{}]",
                self.footnotes
                    .iter()
                    .map(|f| f.display_marker(&self.options))
                    .format(",")
            )?;
        }

        Ok(())
    }
}

impl Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match &self.inner {
            ValueInner::Datum(_) => "Datum",
            ValueInner::Variable(_) => "Variable",
            ValueInner::Text(_) => "Text",
            ValueInner::Markup(_) => "Markup",
            ValueInner::Template(_) => "Template",
            ValueInner::Empty => "Empty",
        };
        write!(f, "{name}:{:?}", self.display(()).to_string())?;
        if let Some(markup) = self.inner.as_markup() {
            write!(f, " (markup: {markup:?})")?;
        }
        if let Some(styling) = &self.styling {
            write!(f, " ({styling:?})")?;
        }
        Ok(())
    }
}

/// A [Format] inside a [Value].
///
/// Most `Value`s contain ordinary [Format]s, but occasionally one will have a
/// special format that is like [Type::F] except that nonzero numbers with
/// magnitude below a small threshold are instead shown in scientific notation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ValueFormat {
    /// Any ordinary format.
    Other(Format),

    /// Displays numbers smaller than [PivotTableStyle::small] in scientific
    /// notation, and otherwise in the enclosed format (which should be
    /// [Type::F] format).
    ///
    /// [PivotTableStyle::small]: super::PivotTableStyle::small
    SmallE(Format),
}

impl ValueFormat {
    /// Returns the inner [Format].
    pub fn inner(&self) -> Format {
        match self {
            ValueFormat::Other(format) => *format,
            ValueFormat::SmallE(format) => *format,
        }
    }

    /// Returns this format as applied to the given `number` with `small` as the
    /// threshold.
    pub fn apply(&self, number: Option<f64>, small: f64) -> Format {
        if let ValueFormat::SmallE(format) = self
            && let Some(number) = number
            && number != 0.0
            && number.abs() < small
        {
            UncheckedFormat::new(Type::E, 40, format.d() as u8).fix()
        } else {
            self.inner()
        }
    }

    /// Returns true if this is [ValueFormat::SmallE].
    pub fn is_small_e(&self) -> bool {
        matches!(self, ValueFormat::SmallE(_))
    }
}

impl From<Format> for ValueFormat {
    fn from(format: Format) -> Self {
        Self::Other(format)
    }
}

impl Serialize for ValueFormat {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            ValueFormat::Other(format) => format.serialize(serializer),
            ValueFormat::SmallE(format) => {
                #[derive(Serialize)]
                struct SmallE(Format);
                SmallE(*format).serialize(serializer)
            }
        }
    }
}

/// A datum and how to display it.
#[derive(Clone, Debug, PartialEq)]
pub struct DatumValue {
    /// The datum.
    pub datum: Datum<WithEncoding<ByteString>>,

    /// The display format.
    pub format: ValueFormat,

    /// Whether to show `value` or `value_label` or both.
    ///
    /// If this is unset, then a higher-level default is used.
    pub show: Option<Show>,

    /// The name of the variable that `value` came from, if any.
    pub variable: Option<String>,

    /// The value label associated with `value`, if any.
    pub value_label: Option<String>,
}

impl Serialize for DatumValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let ValueFormat::Other(format) = self.format
            && format.type_() == Type::F
            && self.variable.is_none()
            && self.value_label.is_none()
        {
            self.datum.serialize(serializer)
        } else {
            let mut s = serializer.serialize_map(None)?;
            s.serialize_entry("datum", &self.datum)?;
            s.serialize_entry("format", &self.format)?;
            if let Some(show) = self.show {
                s.serialize_entry("show", &show)?;
            }
            if let Some(variable) = &self.variable {
                s.serialize_entry("variable", variable)?;
            }
            if let Some(value_label) = &self.value_label {
                s.serialize_entry("value_label", value_label)?;
            }
            s.end()
        }
    }
}

impl DatumValue {
    /// Constructs a new `DatumValue` for `datum`.
    pub fn new<B>(datum: &Datum<B>) -> Self
    where
        B: EncodedString,
    {
        Self {
            datum: datum.cloned(),
            format: ValueFormat::Other(F8_2),
            show: None,
            variable: None,
            value_label: None,
        }
    }

    /// Constructs a new `DatumValue` for `number`.
    pub fn new_number(number: Option<f64>) -> Self {
        Self::new(&Datum::<&str>::Number(number))
    }

    /// Returns this `DatumValue` with the given `format`.
    pub fn with_format(self, format: ValueFormat) -> Self {
        Self { format, ..self }
    }

    /// Writes this value to `f` using the settings in `display`.
    pub fn display<'a>(
        &self,
        display: &DisplayValue<'a>,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        if display.show_value {
            match &self.datum {
                Datum::Number(number) => {
                    let format = self.format.apply(*number, display.small());
                    self.datum
                        .display(format)
                        .with_settings(&display.options.settings)
                        .without_leading_spaces()
                        .fmt(f)?;
                }
                Datum::String(s) => {
                    if self.format.inner().type_() == Type::AHex {
                        write!(f, "{}", s.inner.display_hex())?;
                    } else {
                        f.write_str(&s.as_str())?;
                    }
                }
            }
        }
        if let Some(label) = display.show_label {
            if display.show_value {
                f.write_char(' ')?;
            }
            f.write_str(label)?;
        }
        Ok(())
    }

    /// Returns the decimal point used in the formatted value, if any.
    pub fn decimal(&self) -> Decimal {
        self.datum.display(self.format.inner()).decimal()
    }

    /// Serializes this value to `serializer` in the "bare" manner described for
    /// [BareValue].
    pub fn serialize_bare<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Datum::Number(Some(number)) = &self.datum
            && let number = *number
            && number.trunc() == number
            && number >= -(1i64 << 53) as f64
            && number <= (1i64 << 53) as f64
        {
            Some(number as u64).serialize(serializer)
        } else {
            self.datum.serialize(serializer)
        }
    }
}

/// A variable name.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct VariableValue {
    /// Variable name.
    pub var_name: String,

    /// Variable label, if any.
    pub variable_label: Option<String>,

    /// Whether to show `var_name` or `variable_label` or both.
    ///
    /// If this is unset, then a higher-level default is used.
    pub show: Option<Show>,
}

impl VariableValue {
    fn display(&self, display: &DisplayValue<'_>, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if display.show_value {
            f.write_str(&self.var_name)?;
        }
        if let Some(label) = display.show_label {
            if display.show_value {
                f.write_char(' ')?;
            }
            f.write_str(label)?;
        }
        Ok(())
    }
}

/// A text string.
///
/// A `TextValue` is used for text within a table, such as a title, a column or
/// row heading, or a footnote.  (String data values are better represented as
/// [DatumValue].)
#[derive(Clone, Debug, PartialEq)]
pub struct TextValue {
    /// Whether the text came from the user.
    ///
    /// PSPP can localize text that it writes itself, but not text provided by
    /// the user.
    pub user_provided: bool,

    /// Localized.
    ///
    /// This is the main output string.
    pub localized: String,

    /// English version of the string.
    ///
    /// Only for strings that are not user-provided, and only if it is different
    /// from `localized`.
    pub c: Option<String>,

    /// Identifier.
    ///
    /// Only for strings that are not user-provided, and only if it is different
    /// from `localized`.
    pub id: Option<String>,
}

impl Serialize for TextValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.user_provided && self.c.is_none() && self.id.is_none() {
            serializer.serialize_str(&self.localized)
        } else {
            let mut s = serializer.serialize_struct(
                "TextValue",
                2 + self.c.is_some() as usize + self.id.is_some() as usize,
            )?;
            s.serialize_field("user_provided", &self.user_provided)?;
            s.serialize_field("localized", &self.localized)?;
            if let Some(c) = &self.c {
                s.serialize_field("c", &c)?;
            }
            if let Some(id) = &self.id {
                s.serialize_field("id", &id)?;
            }
            s.end()
        }
    }
}

impl Display for TextValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.localized)
    }
}

impl TextValue {
    /// Returns the localized version of this `TextValue`.
    pub fn localized(&self) -> &str {
        self.localized.as_str()
    }

    /// Returns the English version of this `TextValue`.
    pub fn c(&self) -> &str {
        self.c.as_ref().unwrap_or(&self.localized).as_str()
    }

    /// Returns an identifier for this `TextValue`.
    pub fn id(&self) -> &str {
        self.id.as_ref().unwrap_or(&self.localized).as_str()
    }
}

/// A template with substitutions.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct TemplateValue {
    /// Template string.
    ///
    /// The documentation for [Value] in the PSPP manual describes the template
    /// syntax.
    ///
    /// [Value]: https://pspp.benpfaff.org/manual/spv/light-detail.html#value
    pub localized: String,

    /// Arguments to the template string.
    pub args: Vec<Vec<Value>>,

    /// Optional identifier for the template.
    pub id: Option<String>,
}

impl TemplateValue {
    fn display<'a>(
        &self,
        display: &DisplayValue<'a>,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        #[derive(Copy, Clone, Debug)]
        struct InnerTemplate<'b> {
            template: &'b str,
            escape: char,
        }

        impl<'b> InnerTemplate<'b> {
            fn new(template: &'b str, escape: char) -> Self {
                Self { template, escape }
            }

            fn extract(input: &'b str, escape: char, end: char) -> (Self, &'b str) {
                let mut prev = None;
                for (index, c) in input.char_indices() {
                    if c == end && prev != Some('\\') {
                        return (Self::new(&input[..index], escape), &input[index + 1..]);
                    }
                    prev = Some(c);
                }
                (Self::new(input, escape), "")
            }

            fn expand(
                &self,
                options: &ValueOptions,
                f: &mut std::fmt::Formatter<'_>,
                args: &mut std::slice::Iter<Value>,
            ) -> Result<usize, std::fmt::Error> {
                let mut iter = self.template.chars();

                // Always consume at least 1 argument to avoid infinite loop.
                let mut args_consumed = 1;

                while let Some(c) = iter.next() {
                    match c {
                        '\\' => {
                            let c = iter.next().unwrap_or('\\') as char;
                            let c = if c == 'n' { '\n' } else { c };
                            write!(f, "{c}")?;
                        }
                        c if c == self.escape => {
                            let (index, rest) = consume_int(iter.as_str());
                            iter = rest.chars();
                            if let Some(index) = index.checked_sub(1)
                                && let Some(arg) = args.as_slice().get(index)
                            {
                                args_consumed = args_consumed.max(index + 1);
                                write!(f, "{}", arg.display(options))?;
                            }
                        }
                        c => write!(f, "{c}")?,
                    }
                }
                for _ in 0..args_consumed {
                    args.next();
                }
                Ok(args_consumed)
            }
        }

        fn consume_int(input: &str) -> (usize, &str) {
            let mut n = 0;
            for (index, c) in input.char_indices() {
                match c.to_digit(10) {
                    Some(digit) => n = n * 10 + digit as usize,
                    None => return (n, &input[index..]),
                }
            }
            (n, "")
        }

        // Arguments are formatted without leading zeros for `PCT` and `DOLLAR`.
        // (I don't know why.)
        let mut options = display.options.clone();
        options.settings.leading_zero_pct = false;

        let mut iter = self.localized.chars();
        while let Some(c) = iter.next() {
            match c {
                '\\' => {
                    let c = match iter.next() {
                        None => '\\',
                        Some('n') => '\n',
                        Some(c) => c,
                    };
                    f.write_char(c)?;
                }
                '^' => {
                    let (index, rest) = consume_int(iter.as_str());
                    if let Some(index) = index.checked_sub(1)
                        && let Some(arg) = self.args.get(index)
                        && let Some(arg) = arg.first()
                    {
                        write!(f, "{}", arg.display(&options))?;
                    }
                    iter = rest.chars();
                }
                '[' => {
                    let (a, rest) = InnerTemplate::extract(iter.as_str(), '%', ':');
                    let (b, rest) = InnerTemplate::extract(rest, '^', ':');
                    let (c, rest) = InnerTemplate::extract(rest, '$', ']');
                    let (index, rest) = consume_int(rest);
                    iter = rest.chars();

                    let (first, mid, last) = if a.template.is_empty() {
                        (b, b, b)
                    } else if c.template.is_empty() {
                        (a, b, b)
                    } else {
                        (a, b, c)
                    };
                    if let Some(index) = index.checked_sub(1)
                        && let Some(args) = self.args.get(index)
                    {
                        let mut args = args.iter();
                        let n = first.expand(&options, f, &mut args)?;
                        while args.len() > n {
                            mid.expand(&options, f, &mut args)?;
                        }
                        if args.len() > 0 {
                            last.expand(&options, f, &mut args)?;
                        }
                    }
                }
                c => f.write_char(c)?,
            }
        }
        Ok(())
    }
}

/// Possible content for a [Value].
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ValueInner {
    /// A [Datum] value.
    Datum(
        /// The datum.
        DatumValue,
    ),
    /// A variable name.
    Variable(
        /// The variable.
        VariableValue,
    ),
    /// Plain text.
    Text(
        /// The text.
        TextValue,
    ),
    /// Rich text.
    Markup(
        /// The rich text.
        Markup,
    ),
    /// A template with substitutions.
    Template(
        /// The template.
        TemplateValue,
    ),
    /// An empty value.
    #[default]
    Empty,
}

impl ValueInner {
    /// Returns true if this is a [ValueInner::Empty].
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Returns this value with its display format set to `format`, if it is a
    /// [DatumValue].
    pub fn with_format(mut self, format: impl Into<ValueFormat>) -> Self {
        if let Some(datum_value) = self.as_datum_value_mut() {
            datum_value.format = format.into();
        }
        self
    }

    /// Returns the [Datum] inside this value, if it is a [DatumValue].
    pub fn datum(&self) -> Option<&Datum<WithEncoding<ByteString>>> {
        self.as_datum_value().map(|d| &d.datum)
    }

    /// Returns the [Show] value inside this value, if it has one, or [None]
    /// otherwise.
    fn show(&self) -> Option<Show> {
        match self {
            ValueInner::Datum(DatumValue { show, .. })
            | ValueInner::Variable(VariableValue { show, .. }) => *show,
            _ => None,
        }
    }

    /// Returns the value label or variable label inside this value, if it has
    /// one.
    pub fn label(&self) -> Option<&str> {
        self.value_label().or_else(|| self.variable_label())
    }

    /// Returns the value label inside this value, if it has one.
    fn value_label(&self) -> Option<&str> {
        self.as_datum_value()
            .and_then(|d| d.value_label.as_ref().map(String::as_str))
    }

    /// Returns the variable label inside this value, if it has one.
    fn variable_label(&self) -> Option<&str> {
        self.as_variable_value()
            .and_then(|d| d.variable_label.as_ref().map(String::as_str))
    }

    /// Returns the [DatumValue] inside this value, if it is
    /// [ValueInner::Datum].
    pub fn as_datum_value(&self) -> Option<&DatumValue> {
        match self {
            ValueInner::Datum(datum) => Some(datum),
            _ => None,
        }
    }

    /// Returns the [DatumValue] inside this value, mutably, if it is
    /// [ValueInner::Datum].
    pub fn as_datum_value_mut(&mut self) -> Option<&mut DatumValue> {
        match self {
            ValueInner::Datum(datum) => Some(datum),
            _ => None,
        }
    }

    /// Returns the [VariableValue] inside this value, if it is
    /// [ValueInner::Variable].
    pub fn as_variable_value(&self) -> Option<&VariableValue> {
        match self {
            ValueInner::Variable(variable) => Some(variable),
            _ => None,
        }
    }

    /// Returns the [VariableValue] inside this value, mutably, if it is
    /// [ValueInner::Variable].
    pub fn as_variable_value_mut(&mut self) -> Option<&mut VariableValue> {
        match self {
            ValueInner::Variable(variable) => Some(variable),
            _ => None,
        }
    }

    /// Returns the [Markup] inside this value, if it is [ValueInner::Markup].
    fn as_markup(&self) -> Option<&Markup> {
        match self {
            ValueInner::Markup(markup) => Some(markup),
            _ => None,
        }
    }

    /// Returns an object that will format this value.  Settings on `options`
    /// control whether variable and value labels are included.
    pub fn display(&self, options: impl Into<ValueOptions>) -> DisplayValue<'_> {
        fn interpret_show(
            global_show: impl Fn() -> Show,
            table_show: Option<Show>,
            value_show: Option<Show>,
            label: &str,
        ) -> (bool, Option<&str>) {
            match value_show.or(table_show).unwrap_or_else(global_show) {
                Show::Value => (true, None),
                Show::Label => (false, Some(label)),
                Show::Both => (true, Some(label)),
            }
        }

        let options = options.into();
        let (show_value, show_label) = if let Some(value_label) = self.value_label() {
            interpret_show(
                || Settings::global().show_values,
                options.show_values,
                self.show(),
                value_label,
            )
        } else if let Some(variable_label) = self.variable_label() {
            interpret_show(
                || Settings::global().show_variables,
                options.show_variables,
                self.show(),
                variable_label,
            )
        } else {
            (true, None)
        };
        DisplayValue {
            inner: self,
            subscripts: &[],
            footnotes: &[],
            options,
            show_value,
            show_label,
        }
    }

    /// Constructs a new text `ValueInner` from `s`, which should have been
    /// provided by the user.
    pub fn new_user_text(s: impl Into<String>) -> Self {
        let s: String = s.into();
        if !s.is_empty() {
            Self::Text(TextValue {
                user_provided: true,
                localized: s,
                c: None,
                id: None,
            })
        } else {
            Self::Empty
        }
    }
}

/// Styling inside a [Value].
///
/// Most [Value]s use a default style, so this is a separate [Box]ed structure
/// to save memory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValueStyle {
    /// Cell style.
    pub cell_style: Option<CellStyle>,

    /// Font style.
    pub font_style: Option<FontStyle>,

    /// Subscripts.
    pub subscripts: Vec<String>,

    /// Footnotes.
    pub footnotes: Vec<Arc<Footnote>>,
}

impl ValueStyle {
    /// Returns true if this [ValueStyle] is empty.
    ///
    /// This will return false if the font style exists but is the default font
    /// style, and similarly for the cell style.
    pub fn is_empty(&self) -> bool {
        self.font_style.is_none()
            && self.cell_style.is_none()
            && self.subscripts.is_empty()
            && self.footnotes.is_empty()
    }
}

/// Options for displaying a [Value].
#[derive(Clone, Debug)]
pub struct ValueOptions {
    /// Whether to show values or value labels, or both.
    ///
    /// When this is `None`, a global default is used.
    pub show_values: Option<Show>,

    /// Whether to show variable names or variable labels, or both.
    ///
    /// When this is `None`, a global default is used.
    pub show_variables: Option<Show>,

    /// Numbers whose magnitudes are less than this value are displayed in
    /// scientific notation.  A value of 0 disables this feature.
    pub small: f64,

    /// Where to put the footnote markers.
    pub footnote_marker_type: FootnoteMarkerType,

    /// Settings for formatting [Datum]s.
    pub settings: format::Settings,
}

impl Default for ValueOptions {
    fn default() -> Self {
        Self {
            show_values: None,
            show_variables: None,
            small: 0.0001,
            footnote_marker_type: FootnoteMarkerType::default(),
            settings: Settings::global().formats.clone(),
        }
    }
}

impl From<()> for ValueOptions {
    fn from(_: ()) -> Self {
        ValueOptions::default()
    }
}

impl From<&ValueOptions> for ValueOptions {
    fn from(value: &ValueOptions) -> Self {
        value.clone()
    }
}
