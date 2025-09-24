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

#![warn(dead_code)]
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    fmt::Debug,
    marker::PhantomData,
    num::NonZeroUsize,
    ops::Range,
    sync::Arc,
};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use displaydoc::Display;
use enum_map::{Enum, EnumMap};
use hashbrown::HashSet;
use indexmap::IndexMap;
use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::{
    data::{Datum, EncodedString},
    format::{self, Decimal, F8_0, F40_2, Type, UncheckedFormat},
    output::pivot::{
        self, Axis2, Axis3, Category, CategoryLocator, Dimension, Group, Leaf, Length, PivotTable,
        look::{self, Area, AreaStyle, CellStyle, Color, HorzAlign, Look, RowParity, VertAlign},
        value::{Value, ValueFormat},
    },
    spv::read::light::decode_format,
};

/// Interprets this data value as a [Format], first by looking it up in
/// `format_map` and otherwise by interpreting it as a [Format] directly.
///
/// This should probably be a method on some hypothetical FormatMap.
pub fn datum_as_format(datum: &Datum<String>) -> crate::format::Format {
    let f = match datum {
        Datum::Number(Some(number)) => *number as i64,
        Datum::Number(None) => 0,
        Datum::String(s) => s.parse().unwrap_or_default(),
    };
    decode_format(f as u32, &mut |_| () /*XXX*/).inner()
}

/// Returns this data value interpreted using `format`.
fn datum_as_pivot_value(datum: &Datum<String>, format: crate::format::Format) -> Value {
    if format.type_().category() == crate::format::Category::Date
        && let Some(s) = datum.as_string()
        && let Ok(date_time) = NaiveDateTime::parse_from_str(s.as_str(), "%Y-%m-%dT%H:%M:%S%.3f")
    {
        Value::new_date(date_time)
    } else if format.type_().category() == crate::format::Category::Time
        && let Some(s) = datum.as_string()
        && let Ok(time) = NaiveTime::parse_from_str(s.as_str(), "%H:%M:%S%.3f")
    {
        Value::new_time(time)
    } else if !datum.is_sysmis() {
        Value::new_datum(&datum)
    } else {
        Value::new_empty()
    }
    .with_format(format)
}

#[derive(Debug)]
struct Ref<T> {
    references: String,
    _phantom: PhantomData<T>,
}

impl<T> Ref<T> {
    fn get<'a>(&self, table: &HashMap<&str, &'a T>) -> Option<&'a T> {
        table.get(self.references.as_str()).map(|v| &**v)
    }
}

impl<'de, T> Deserialize<'de> for Ref<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self {
            references: String::deserialize(deserializer)?,
            _phantom: PhantomData,
        })
    }
}

#[derive(Clone, Debug, Default)]
struct Map(HashMap<OrderedFloat<f64>, Datum<String>>);

impl Map {
    fn apply(&self, data: &mut Vec<Datum<String>>) {
        for value in data {
            if let Datum::Number(Some(number)) = value
                && let Some(to) = self.0.get(&OrderedFloat(*number))
            {
                *value = to.clone();
            }
        }
    }
}

/// A warning decoding a legacy XML member.
#[derive(Clone, Debug, Display, thiserror::Error)]
pub enum LegacyXmlWarning {
    /// Table has no data.
    MissingData,

    /// Table has variables with unresolved dependencies: {0:?}.
    UnresolvedDependencies(Vec<String>),

    /// Derived variable {variable:?} has unsupported value expression {value:?}.
    UnsupportedValue {
        /// Name of derived variable.
        variable: String,
        /// Value expression.
        value: String,
    },

    /// Unsupported applyToConverse.
    UnsupportedApplyToConverse,

    /// Invalid creation date {0:?}.
    InvalidCreationDate(String),
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Visualization {
    /// In format `YYYY-MM-DD`.
    #[serde(rename = "@date")]
    date: String,

    // Locale used for output, e.g. `en-US`.
    #[serde(rename = "@lang")]
    lang: String,

    /// Localized title of the pivot table.
    ///
    /// This is repeated in the table's labels, and that form supports
    /// footnotes, so we don't read this one.
    #[serde(rename = "@name")]
    _name: String,

    /// Base style for the pivot table.
    #[serde(rename = "@style")]
    _style: Ref<Style>,

    #[serde(rename = "$value")]
    children: Vec<VisChild>,
}

impl Visualization {
    fn decode_labels(&self, graph: &Graph) -> (Option<Value>, Option<Value>, pivot::Footnotes) {
        let mut labels = EnumMap::from_fn(|_| Vec::new());
        for child in &self.children {
            match child {
                VisChild::LabelFrame(label_frame) => {
                    if let Some(label) = &label_frame.label
                        && let Some(purpose) = label.purpose
                    {
                        labels[purpose].push(label);
                    }
                }
                VisChild::Container(c) => {
                    for label_frame in &c.label_frames {
                        if let Some(label) = &label_frame.label
                            && let Some(purpose) = label.purpose
                        {
                            labels[purpose].push(label);
                        }
                    }
                }
                _ => (),
            }
        }
        let caption_labels = if !labels[Purpose::SubTitle].is_empty() {
            &labels[Purpose::SubTitle]
        } else {
            &labels[Purpose::Footnote]
        };

        let footnotes = self.decode_footnotes(graph, &labels[Purpose::Footnote]);
        (
            LabelFrame::decode_label(&labels[Purpose::Title], &footnotes),
            LabelFrame::decode_label(caption_labels, &footnotes),
            footnotes,
        )
    }
    fn decode_date(&self, warn: &mut dyn FnMut(LegacyXmlWarning)) -> Option<NaiveDateTime> {
        match NaiveDate::parse_from_str(&self.date, "%Y-%m-%d") {
            Ok(date) => Some(date.into()),
            Err(_) => {
                warn(LegacyXmlWarning::InvalidCreationDate(self.date.clone()));
                None
            }
        }
    }
    pub fn decode_series(
        &self,
        data: IndexMap<String, IndexMap<String, Vec<Datum<String>>>>,
        warn: &mut dyn FnMut(LegacyXmlWarning),
    ) -> BTreeMap<&str, Series> {
        let mut variables: Vec<&dyn Variable> = self
            .children
            .iter()
            .filter_map(|child| child.variable())
            .collect();
        let mut series = BTreeMap::<&str, Series>::new();
        while !variables.is_empty() {
            let n = variables.len();
            variables.retain(|variable| !variable.decode(&data, &mut series, warn));
            if n == variables.len() {
                warn(LegacyXmlWarning::UnresolvedDependencies(
                    variables
                        .iter()
                        .map(|variable| variable.name().into())
                        .collect(),
                ));
                break;
            }
        }
        series
    }

    /// Decodes the table of footnotes from the `graph` and the collection of
    /// all the footnote labels.  Any [Value] might refer to footnotes, so it's
    /// important to process the footnotes early to ensure that those references
    /// can be resolved.
    fn decode_footnotes(&self, graph: &Graph, footnote_labels: &[&Label]) -> pivot::Footnotes {
        let mut footnote_builder = BTreeMap::<usize, Footnote>::new();
        if let Some(f) = &graph.interval.footnotes {
            f.decode(&mut footnote_builder);
        }
        for child in &graph.interval.labeling.children {
            if let LabelingChild::Footnotes(f) = child {
                f.decode(&mut footnote_builder);
            }
        }
        for label in footnote_labels {
            for (i, text) in label.text().iter().enumerate() {
                if let DecodedText::FootnoteDefinition { index, text } = text.decode() {
                    let entry = footnote_builder.entry(index).or_default();
                    if i % 2 == 1 {
                        entry.content = text.strip_suffix('\n').unwrap_or(text).into();
                    } else {
                        entry.marker =
                            Some(text.trim_end().strip_suffix('.').unwrap_or(text).into());
                    }
                }
            }
        }
        let mut footnotes = Vec::new();
        for (index, footnote) in footnote_builder {
            while footnotes.len() < index {
                footnotes.push(pivot::Footnote::default());
            }
            footnotes.push(
                pivot::Footnote::new(footnote.content)
                    .with_marker(footnote.marker.map(|s| Value::new_user_text(s))),
            );
        }
        pivot::Footnotes::from_iter(footnotes)
    }

    fn decode_dimensions<'a>(
        &self,
        graph: &Graph,
        series: &'a BTreeMap<&str, Series>,
        footnotes: &pivot::Footnotes,
        styles: &HashMap<&str, &Style>,
    ) -> (Vec<Dim<'a>>, Vec<usize>) {
        let axes = graph
            .facet_layout
            .children
            .iter()
            .filter_map(|child| child.facet_level())
            .map(|facet_level| (facet_level.level, &facet_level.axis))
            .collect::<HashMap<_, _>>();
        let mut dims = Vec::new();
        let mut level_ofs = 1;
        let mut current_layer = Vec::new();

        for (axis, dimension) in graph.faceting.dimensions() {
            let dim_series = decode_axis_dimensions(
                dimension.iter().copied(),
                &series,
                &axes,
                &styles,
                axis,
                &footnotes,
                level_ofs,
                &mut dims,
            );
            if axis == Axis3::Z {
                current_layer = dim_series
                    .into_iter()
                    .map(|series| {
                        let name = &series.name;
                        let coordinate = graph.faceting.layer_value(&name).unwrap();
                        series
                            .coordinate_to_index
                            .borrow()
                            .get(&coordinate)
                            .unwrap()
                            .as_leaf()
                            .unwrap()
                    })
                    .collect();
            }
            level_ofs += dimension.len();
        }

        (dims, current_layer)
    }

    fn graph(&self) -> Result<&Graph, super::Error> {
        for child in &self.children {
            match child {
                VisChild::Graph(g) => return Ok(g),
                _ => (),
            }
        }
        Err(super::Error::LegacyMissingGraph)
    }

    pub fn decode(
        &self,
        binary_data: IndexMap<String, IndexMap<String, Vec<Datum<String>>>>,
        look: Look,
        warn: &mut dyn FnMut(LegacyXmlWarning),
    ) -> Result<PivotTable, super::Error> {
        let graph = self.graph()?;
        let (title, caption, footnotes) = self.decode_labels(graph);

        let series = self.decode_series(binary_data, warn);
        let styles = self
            .children
            .iter()
            .filter_map(|child| child.style())
            .filter_map(|style| style.id.as_ref().map(|id| (id.as_str(), style)))
            .collect::<HashMap<_, _>>();
        let (mut dims, current_layer) = self.decode_dimensions(graph, &series, &footnotes, &styles);

        let mut data = graph.decode_data(&footnotes, &dims, &series, warn);
        graph.decode_styles(
            &look, &series, &mut dims, &mut data, &footnotes, &styles, warn,
        );

        Ok(PivotTable::new(
            dims.into_iter()
                .map(|dim| {
                    (
                        dim.axis,
                        Dimension::new(dim.root).with_hide_all_labels(dim.hide_all_labels),
                    )
                })
                .collect::<Vec<_>>(),
        )
        .with_look(Arc::new(look))
        .with_footnotes(footnotes)
        .with_data(data)
        .with_layer(&current_layer)
        .with_decimal(Decimal::for_lang(&self.lang))
        .with_date(self.decode_date(warn))
        .with_optional_title(title)
        .with_optional_caption(caption))
    }
}

pub struct Series {
    pub name: String,
    label: Option<String>,
    categories: Vec<Option<usize>>,
    pub values: Vec<Datum<String>>,
    affixes: Vec<Affix>,
    coordinate_to_index: RefCell<BTreeMap<usize, CategoryLocator>>,
    dimension_index: Cell<Option<usize>>,
}

impl Debug for Series {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Series")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Series {
    fn new(name: String, categories: Vec<Option<usize>>, values: Vec<Datum<String>>) -> Self {
        Self {
            name,
            label: None,
            categories,
            values,
            affixes: Vec::new(),
            coordinate_to_index: Default::default(),
            dimension_index: Default::default(),
        }
    }
    fn with_label(self, label: Option<String>) -> Self {
        Self { label, ..self }
    }
    fn with_affixes(self, affixes: Vec<Affix>) -> Self {
        Self { affixes, ..self }
    }

    fn new_name(&self, datum: &Datum<String>, footnotes: &pivot::Footnotes) -> Value {
        let mut name = Value::new_datum(datum);
        for affix in &self.affixes {
            if let Some(footnote) = footnotes.get(affix.defines_reference.get() - 1) {
                name.add_footnote(footnote);
            }
        }
        name
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum VisChild {
    SourceVariable(SourceVariable),
    DerivedVariable(DerivedVariable),
    Graph(Graph),
    LabelFrame(LabelFrame),
    Container(Container),
    Style(Style),
    #[serde(other)]
    Other,
}

impl VisChild {
    fn variable(&self) -> Option<&dyn Variable> {
        match self {
            VisChild::SourceVariable(source_variable) => Some(source_variable),
            VisChild::DerivedVariable(derived_variable) => Some(derived_variable),
            _ => None,
        }
    }
    fn style(&self) -> Option<&Style> {
        match self {
            Self::Style(style) => Some(style),
            _ => None,
        }
    }
}

trait Variable {
    fn decode<'a>(
        &'a self,
        data: &IndexMap<String, IndexMap<String, Vec<Datum<String>>>>,
        series: &mut BTreeMap<&'a str, Series>,
        warn: &mut dyn FnMut(LegacyXmlWarning),
    ) -> bool;
    fn name(&self) -> &str;
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SourceVariable {
    #[serde(rename = "@id")]
    id: String,

    /// The `source-name` in the `tableData.bin` member.
    #[serde(rename = "@source")]
    source: String,

    /// The name of a variable within the source, corresponding to the
    /// `variable-name` in the `tableData.bin` member.
    #[serde(rename = "@sourceName")]
    variable: String,

    /// Variable label, if any.
    #[serde(rename = "@label")]
    label: Option<String>,

    /// A variable whose string values correspond one-to-one with the values of
    /// this variable and are suitable as value labels.
    #[serde(rename = "@labelVariable")]
    label_variable: Option<Ref<SourceVariable>>,

    format: Option<Format>,
    string_format: Option<StringFormat>,
}

impl SourceVariable {
    fn affixes(&self) -> &[Affix] {
        if let Some(format) = &self.format {
            &format.affixes
        } else if let Some(string_format) = &self.string_format {
            &string_format.affixes
        } else {
            &[]
        }
    }
}
impl Variable for SourceVariable {
    fn decode<'a>(
        &'a self,
        data: &IndexMap<String, IndexMap<String, Vec<Datum<String>>>>,
        series: &mut BTreeMap<&'a str, Series>,
        _warn: &mut dyn FnMut(LegacyXmlWarning),
    ) -> bool {
        let label_series = if let Some(label_variable) = &self.label_variable {
            if let Some(label_series) = series.get(label_variable.references.as_str()) {
                Some(label_series)
            } else {
                return false;
            }
        } else {
            None
        };

        let (categories, mut data) = if let Some(source) = data.get(&self.source)
            && let Some(values) = source.get(&self.variable)
        {
            let categories = values
                .iter()
                .map(|value| {
                    value
                        .as_number()
                        .flatten()
                        .and_then(|v| (v >= 0.0 && v < usize::MAX as f64).then_some(v as usize))
                })
                .collect();
            (categories, values.clone())
        } else {
            (Vec::new(), Vec::new())
        };
        if let Some(format) = &self.format {
            format.mapping().apply(&mut data);
        } else if let Some(string_format) = &self.string_format {
            string_format.mapping().apply(&mut data);
        };
        if let Some(label_series) = label_series {
            let format = self.format.as_ref().map_or(F8_0, |f| f.decode());
            data = label_series
                .values
                .iter()
                .map(|label| {
                    if label.is_number() {
                        Datum::String(label.display(format).with_stretch().to_string())
                    } else {
                        label.clone()
                    }
                })
                .collect();
        }
        series.insert(
            &self.id,
            Series::new(self.id.clone(), categories, data)
                .with_affixes(Vec::from(self.affixes()))
                .with_label(self.label.clone()),
        );
        true
    }
    fn name(&self) -> &str {
        &self.variable
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DerivedVariable {
    #[serde(rename = "@id")]
    id: String,

    /// An expression that defines the variable's value.
    #[serde(rename = "@value")]
    value: String,
    format: Option<Format>,
    string_format: Option<StringFormat>,
    #[serde(default, rename = "valueMapEntry")]
    value_map: Vec<ValueMapEntry>,
}

impl DerivedVariable {
    fn mapping(&self) -> Map {
        Map(self
            .value_map
            .iter()
            .flat_map(|vme| {
                vme.from
                    .split(';')
                    .filter_map(|from| from.trim().parse::<f64>().ok())
                    .map(|from| {
                        (
                            OrderedFloat(from),
                            if let Ok(to) = vme.to.trim().parse::<f64>() {
                                Datum::Number(Some(to))
                            } else {
                                Datum::String(vme.to.clone())
                            },
                        )
                    })
            })
            .collect())
    }
}

impl Variable for DerivedVariable {
    fn decode<'a>(
        &'a self,
        _data: &IndexMap<String, IndexMap<String, Vec<Datum<String>>>>,
        series: &mut BTreeMap<&'a str, Series>,
        warn: &mut dyn FnMut(LegacyXmlWarning),
    ) -> bool {
        let mut values = if self.id == "column" || self.id == "row" {
            vec![]
        } else if let Some(rest) = self.id.strip_prefix("dimension")
            && rest.parse::<usize>().is_ok()
        {
            vec![]
        } else if self.value == "constant(0)" {
            /// All the series have the same length, except for series with
            /// length zero.  This returns the length of the first nonempty
            /// series, or `None` if there isn't one yet.
            fn series_len(series: &mut BTreeMap<&str, Series>) -> Option<usize> {
                series.values().find_map(|series| {
                    if !series.values.is_empty() {
                        Some(series.values.len())
                    } else {
                        None
                    }
                })
            }

            if let Some(n_values) = series_len(series) {
                (0..n_values).map(|_| Datum::Number(Some(0.0))).collect()
            } else {
                return false;
            }
        } else if self.value.starts_with("constant") {
            vec![]
        } else if let Some(rest) = self.value.strip_prefix("map(")
            && let Some(var_name) = rest.strip_suffix(")")
        {
            let Some(dependency) = series.get(var_name) else {
                return false;
            };
            dependency.values.clone()
        } else {
            warn(LegacyXmlWarning::UnsupportedValue {
                variable: self.id.clone(),
                value: self.value.clone(),
            });
            vec![]
        };
        self.mapping().apply(&mut values);
        if let Some(format) = &self.format {
            format.mapping().apply(&mut values);
        } else if let Some(string_format) = &self.string_format {
            string_format.mapping().apply(&mut values);
        };
        series.insert(
            &self.id,
            Series::new(
                self.id.clone(),
                (0..values.len()).map(|_| Some(0)).collect(),
                values,
            ),
        );
        true
    }
    fn name(&self) -> &str {
        &self.id
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct VariableReference {
    #[serde(rename = "@ref")]
    reference: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct StringFormat {
    #[serde(default, rename = "relabel")]
    relabels: Vec<Relabel>,
    #[serde(default, rename = "affix")]
    affixes: Vec<Affix>,
}

impl StringFormat {
    fn mapping(&self) -> Map {
        Map(self
            .relabels
            .iter()
            .map(|relabel| {
                (
                    OrderedFloat(relabel.from),
                    Datum::String(relabel.to.clone()),
                )
            })
            .collect())
    }
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
struct Format {
    #[serde(rename = "@baseFormat")]
    base_format: Option<BaseFormat>,
    #[serde(rename = "@mdyOrder")]
    mdy_order: Option<MdyOrder>,
    #[serde(rename = "@showQuarter")]
    show_quarter: Option<bool>,
    #[serde(rename = "@quarterPrefix")]
    year_abbreviation: Option<bool>,
    #[serde(rename = "@monthFormat")]
    month_format: Option<MonthFormat>,
    #[serde(rename = "@showWeek")]
    show_week: Option<bool>,
    #[serde(rename = "@showDay")]
    show_day: Option<bool>,
    #[serde(rename = "@showHour")]
    show_hour: Option<bool>,
    #[serde(rename = "@showSecond")]
    show_second: Option<bool>,
    #[serde(rename = "@showMillis")]
    show_millis: Option<bool>,
    #[serde(rename = "@maximumFractionDigits")]
    maximum_fraction_digits: Option<i64>,
    #[serde(rename = "@useGrouping")]
    use_grouping: Option<bool>,
    #[serde(rename = "@scientific")]
    scientific: Option<Scientific>,
    #[serde(default, rename = "@prefix")]
    prefix: String,
    #[serde(default, rename = "@suffix")]
    suffix: String,
    #[serde(rename = "@tryStringsAsNumbers", default)]
    try_strings_as_numbers: bool,
    #[serde(default, rename = "relabel")]
    relabels: Vec<Relabel>,
    #[serde(default, rename = "affix")]
    affixes: Vec<Affix>,
}

impl Format {
    fn mapping(&self) -> Map {
        let format = self.decode();
        Map(self
            .relabels
            .iter()
            .map(|relabel| {
                let value = match relabel.to.trim().parse::<f64>().ok() {
                    Some(to) if self.try_strings_as_numbers => Datum::Number(Some(to)),
                    Some(to) => Datum::String(
                        Datum::<String>::Number(Some(to))
                            .display(format)
                            .with_stretch()
                            .to_string(),
                    ),
                    None => Datum::String(relabel.to.clone()),
                };
                (OrderedFloat(relabel.from), value)
            })
            .collect())
    }

    fn decode(&self) -> crate::format::Format {
        if self.base_format.is_some() {
            SignificantDateTimeFormat::from(self).decode()
        } else {
            SignificantNumberFormat::from(self).decode()
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct NumberFormat {
    #[serde(rename = "@maximumFractionDigits")]
    maximum_fraction_digits: Option<i64>,
    #[serde(rename = "@useGrouping")]
    use_grouping: Option<bool>,
    #[serde(rename = "@scientific")]
    scientific: Option<Scientific>,
    #[serde(default, rename = "@prefix")]
    prefix: String,
    #[serde(default, rename = "@suffix")]
    suffix: String,
    #[serde(default, rename = "affix")]
    affixes: Vec<Affix>,
}

struct SignificantNumberFormat<'a> {
    scientific: Option<Scientific>,
    prefix: &'a str,
    suffix: &'a str,
    use_grouping: Option<bool>,
    maximum_fraction_digits: Option<i64>,
}

impl<'a> From<&'a NumberFormat> for SignificantNumberFormat<'a> {
    fn from(value: &'a NumberFormat) -> Self {
        Self {
            scientific: value.scientific,
            prefix: &value.prefix,
            suffix: &value.suffix,
            use_grouping: value.use_grouping,
            maximum_fraction_digits: value.maximum_fraction_digits,
        }
    }
}

impl<'a> From<&'a Format> for SignificantNumberFormat<'a> {
    fn from(value: &'a Format) -> Self {
        Self {
            scientific: value.scientific,
            prefix: &value.prefix,
            suffix: &value.suffix,
            use_grouping: value.use_grouping,
            maximum_fraction_digits: value.maximum_fraction_digits,
        }
    }
}

impl<'a> SignificantNumberFormat<'a> {
    fn decode(&self) -> crate::format::Format {
        let type_ = if self.scientific == Some(Scientific::True) {
            Type::E
        } else if self.prefix == "$" {
            Type::Dollar
        } else if self.suffix == "%" {
            Type::Pct
        } else if self.use_grouping == Some(true) {
            Type::Comma
        } else {
            Type::F
        };
        let d = match self.maximum_fraction_digits {
            Some(d) if (0..=15).contains(&d) => d,
            _ => 2,
        };
        UncheckedFormat {
            type_,
            w: 40,
            d: d as u8,
        }
        .fix()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DateTimeFormat {
    #[serde(rename = "@baseFormat")]
    base_format: BaseFormat,
    #[serde(rename = "@mdyOrder")]
    mdy_order: Option<MdyOrder>,
    #[serde(rename = "@showQuarter")]
    show_quarter: Option<bool>,
    #[serde(rename = "@yearAbbreviation")]
    year_abbreviation: Option<bool>,
    #[serde(rename = "@monthFormat")]
    month_format: Option<MonthFormat>,
    #[serde(rename = "@showWeek")]
    show_week: Option<bool>,
    #[serde(rename = "@showDay")]
    show_day: Option<bool>,
    #[serde(rename = "@showHour")]
    show_hour: Option<bool>,
    #[serde(rename = "@showSecond")]
    show_second: Option<bool>,
    #[serde(rename = "@showMillis")]
    show_millis: Option<bool>,
    #[serde(default, rename = "affix")]
    affixes: Vec<Affix>,
}

struct SignificantDateTimeFormat {
    base_format: Option<BaseFormat>,
    show_quarter: Option<bool>,
    show_week: Option<bool>,
    show_day: Option<bool>,
    show_hour: Option<bool>,
    show_second: Option<bool>,
    show_millis: Option<bool>,
    mdy_order: Option<MdyOrder>,
    month_format: Option<MonthFormat>,
    year_abbreviation: Option<bool>,
}

impl From<&Format> for SignificantDateTimeFormat {
    fn from(value: &Format) -> Self {
        Self {
            base_format: value.base_format,
            show_quarter: value.show_quarter,
            show_week: value.show_week,
            show_day: value.show_day,
            show_hour: value.show_hour,
            show_second: value.show_second,
            show_millis: value.show_millis,
            mdy_order: value.mdy_order,
            month_format: value.month_format,
            year_abbreviation: value.year_abbreviation,
        }
    }
}
impl From<&DateTimeFormat> for SignificantDateTimeFormat {
    fn from(value: &DateTimeFormat) -> Self {
        Self {
            base_format: Some(value.base_format),
            show_quarter: value.show_quarter,
            show_week: value.show_week,
            show_day: value.show_day,
            show_hour: value.show_hour,
            show_second: value.show_second,
            show_millis: value.show_millis,
            mdy_order: value.mdy_order,
            month_format: value.month_format,
            year_abbreviation: value.year_abbreviation,
        }
    }
}
impl SignificantDateTimeFormat {
    fn decode(&self) -> crate::format::Format {
        let type_ = match self.base_format {
            Some(BaseFormat::Date) => {
                let type_ = if self.show_quarter == Some(true) {
                    Type::QYr
                } else if self.show_week == Some(true) {
                    Type::WkYr
                } else {
                    match (self.mdy_order, self.month_format) {
                        (Some(MdyOrder::DayMonthYear), Some(MonthFormat::Number)) => Type::EDate,
                        (Some(MdyOrder::DayMonthYear), Some(MonthFormat::PaddedNumber)) => {
                            Type::EDate
                        }
                        (Some(MdyOrder::DayMonthYear), _) => Type::Date,
                        (Some(MdyOrder::YearMonthDay), _) => Type::SDate,
                        _ => Type::ADate,
                    }
                };
                let mut w = type_.min_width();
                if self.year_abbreviation != Some(true) {
                    w += 2;
                };
                return UncheckedFormat { type_, w, d: 0 }.try_into().unwrap();
            }
            Some(BaseFormat::DateTime) => {
                if self.mdy_order == Some(MdyOrder::YearMonthDay) {
                    Type::YmdHms
                } else {
                    Type::DateTime
                }
            }
            _ => {
                if self.show_day == Some(true) {
                    Type::DTime
                } else if self.show_hour == Some(true) {
                    Type::Time
                } else {
                    Type::MTime
                }
            }
        };
        date_time_format(type_, self.show_second, self.show_millis)
    }
}

impl DateTimeFormat {
    fn decode(&self) -> crate::format::Format {
        SignificantDateTimeFormat::from(self).decode()
    }
}

fn date_time_format(
    type_: Type,
    show_second: Option<bool>,
    show_millis: Option<bool>,
) -> crate::format::Format {
    let mut w = type_.min_width();
    let mut d = 0;
    if show_second == Some(true) {
        w += 3;
        if show_millis == Some(true) {
            d = 3;
            w += d as u16 + 1;
        }
    }
    UncheckedFormat { type_, w, d }.try_into().unwrap()
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ElapsedTimeFormat {
    #[serde(rename = "@showDay")]
    show_day: Option<bool>,
    #[serde(rename = "@showHour")]
    show_hour: Option<bool>,
    #[serde(rename = "@showSecond")]
    show_second: Option<bool>,
    #[serde(rename = "@showMillis")]
    show_millis: Option<bool>,
    #[serde(default, rename = "affix")]
    affixes: Vec<Affix>,
}

impl ElapsedTimeFormat {
    fn decode(&self) -> crate::format::Format {
        let type_ = if self.show_day == Some(true) {
            Type::DTime
        } else if self.show_hour == Some(true) {
            Type::Time
        } else {
            Type::MTime
        };
        date_time_format(type_, self.show_second, self.show_millis)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum BaseFormat {
    Date,
    Time,
    DateTime,
    ElapsedTime,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum MdyOrder {
    DayMonthYear,
    MonthDayYear,
    YearMonthDay,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum MonthFormat {
    Long,
    Short,
    Number,
    PaddedNumber,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Scientific {
    OnlyForSmall,
    WhenNeeded,
    True,
    False,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Affix {
    /// A reference to a footnote with the given 1-based index.
    #[serde(rename = "@definesReference")]
    defines_reference: NonZeroUsize,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Relabel {
    #[serde(rename = "@from")]
    from: f64,
    #[serde(rename = "@to")]
    to: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ValueMapEntry {
    #[serde(rename = "@from")]
    from: String,
    #[serde(rename = "@to")]
    to: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Style {
    #[serde(rename = "@id")]
    id: Option<String>,

    /// The text color or, in some cases, background color.
    #[serde(rename = "@color")]
    color: Option<Color>,

    #[serde(rename = "@font-size")]
    font_size: Option<String>,

    #[serde(rename = "@font-weight")]
    font_weight: Option<FontWeight>,

    #[serde(rename = "@font-style")]
    font_style: Option<FontStyle>,

    #[serde(rename = "@font-underline")]
    font_underline: Option<FontUnderline>,

    #[serde(rename = "@textAlignment")]
    text_alignment: Option<TextAlignment>,

    #[serde(rename = "@labelLocationVertical")]
    label_location_vertical: Option<LabelLocation>,

    #[serde(rename = "@visible")]
    visible: Option<bool>,

    #[serde(rename = "@decimal-offset")]
    decimal_offset: Option<Length>,
}

impl Style {
    fn apply_to_value(
        value: &mut Value,
        sf: Option<&SetFormat>,
        fg: Option<&Style>,
        bg: Option<&Style>,
        base_style: &AreaStyle,
        footnotes: &pivot::Footnotes,
        has_cell_footnotes: bool,
    ) {
        if let Some(sf) = sf {
            if let Some(child) = &sf.child
                && let Some(format) = child.decode_format()
                && let Some(datum_value) = value.inner.as_datum_value_mut()
            {
                match &datum_value.datum {
                    Datum::Number(_) => {
                        datum_value.format = ValueFormat::Other(format);
                    }
                    Datum::String(string) => {
                        if format.type_().category() == format::Category::Date
                            && let Ok(date_time) = NaiveDateTime::parse_from_str(
                                &string.as_str(),
                                "%Y-%m-%dT%H:%M:%S%.3f",
                            )
                        {
                            value.inner = Value::new_date(date_time).with_format(format).inner;
                        } else if format.type_().category() == format::Category::Time
                            && let Ok(time) =
                                NaiveTime::parse_from_str(&string.as_str(), "%H:%M:%S%.3f")
                        {
                            value.inner = Value::new_time(time)
                                .with_format(ValueFormat::Other(format))
                                .inner;
                        } else if let Ok(number) = string.as_str().parse::<f64>() {
                            value.inner = Value::new_number(Some(number)).with_format(format).inner;
                        }
                    }
                }
            }
            if let Some(child) = &sf.child
                && !has_cell_footnotes
            {
                value.clear_footnotes();
                for affix in child.affixes() {
                    if let Some(footnote) = footnotes.get(affix.defines_reference.get() - 1) {
                        value.add_footnote(footnote);
                    }
                }
            }
        }

        if fg.is_some() || bg.is_some() {
            let styling = value.styling_mut();
            let font_style = styling
                .font_style
                .get_or_insert_with(|| base_style.font_style.clone());
            let cell_style = styling
                .cell_style
                .get_or_insert_with(|| base_style.cell_style.clone());
            Self::decode(fg, bg, cell_style, font_style);
        }
    }

    fn decode(
        fg: Option<&Style>,
        bg: Option<&Style>,
        cell_style: &mut CellStyle,
        font_style: &mut look::FontStyle,
    ) {
        Self::decode_font_style(fg, bg, font_style);
        Self::decode_cell_style(fg, cell_style);
    }

    fn decode_font_style(
        fg: Option<&Style>,
        bg: Option<&Style>,
        font_style: &mut look::FontStyle,
    ) -> bool {
        let mut updated = false;
        if let Some(fg) = fg {
            if let Some(weight) = fg.font_weight {
                font_style.bold = weight.is_bold();
                updated = true;
            }
            if let Some(style) = fg.font_style {
                font_style.italic = style.is_italic();
                updated = true;
            }
            if let Some(underline) = fg.font_underline {
                font_style.underline = underline.is_underline();
                updated = true;
            }
            if let Some(color) = fg.color {
                font_style.fg = color;
                updated = true;
            }
            if let Some(font_size) = &fg.font_size {
                if let Ok(size) = font_size
                    .trim_end_matches(|c: char| c.is_alphabetic())
                    .parse()
                {
                    font_style.size = size;
                    updated = true;
                } else {
                    // XXX warn?
                }
            }
        }
        if let Some(bg) = bg
            && let Some(color) = bg.color
        {
            font_style.bg = color;
            updated = true;
        }
        updated
    }

    fn decode_cell_style(fg: Option<&Style>, cell_style: &mut CellStyle) -> bool {
        let mut updated = false;
        if let Some(fg) = fg {
            if let Some(alignment) = fg.text_alignment {
                cell_style.horz_align = alignment.as_horz_align(fg.decimal_offset);
                updated = true;
            }
            if let Some(label_local_vertical) = fg.label_location_vertical {
                cell_style.vert_align = label_local_vertical.into();
                updated = true;
            }
            // XXX margins?
        }
        updated
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FontWeight {
    Regular,
    Bold,
}

impl FontWeight {
    fn is_bold(&self) -> bool {
        *self == Self::Bold
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FontStyle {
    Regular,
    Italic,
}

impl FontStyle {
    fn is_italic(&self) -> bool {
        *self == Self::Italic
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FontUnderline {
    None,
    Underline,
}

impl FontUnderline {
    fn is_underline(&self) -> bool {
        *self == Self::Underline
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum TextAlignment {
    Left,
    Right,
    Center,
    Decimal,
    Mixed,
}

impl TextAlignment {
    fn as_horz_align(&self, decimal_offset: Option<Length>) -> Option<HorzAlign> {
        match self {
            TextAlignment::Left => Some(HorzAlign::Left),
            TextAlignment::Right => Some(HorzAlign::Right),
            TextAlignment::Center => Some(HorzAlign::Center),
            TextAlignment::Decimal => Some(HorzAlign::Decimal {
                offset: decimal_offset.unwrap_or_default().as_px_f64(),
            }),
            TextAlignment::Mixed => None,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum LabelLocation {
    Positive,
    Negative,
    Center,
}

impl From<LabelLocation> for VertAlign {
    fn from(value: LabelLocation) -> Self {
        match value {
            LabelLocation::Positive => VertAlign::Top,
            LabelLocation::Negative => VertAlign::Bottom,
            LabelLocation::Center => VertAlign::Middle,
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Graph {
    faceting: Faceting,
    facet_layout: FacetLayout,
    interval: Interval,
}

impl Graph {
    fn decode_data(
        &self,
        footnotes: &pivot::Footnotes,
        dims: &[Dim],
        series: &BTreeMap<&str, Series>,
        warn: &mut dyn FnMut(LegacyXmlWarning),
    ) -> HashMap<Vec<usize>, Value> {
        let Some(cell) = series.get("cell") else {
            warn(LegacyXmlWarning::MissingData);
            return HashMap::default();
        };

        let markers = if let Some(markers) = self.interval.footnotes(false)
            && let Some(series) = series.get(markers.variable.as_str())
        {
            Some((
                series,
                markers
                    .mappings
                    .iter()
                    .map(|m| (m.from.get(), m.to.as_str()))
                    .collect::<HashMap<_, _>>(),
            ))
        } else {
            None
        };

        let mut data = HashMap::new();
        let mut coords = Vec::with_capacity(dims.len());
        let cell_formats = self.interval.labeling.decode_format_map(&series);
        let cell_footnotes = series.get("footnotes");
        'outer: for (i, cell) in cell.values.iter().enumerate() {
            coords.clear();
            for dim in dims {
                if let Some(Some(coordinate)) = dim.series.categories.get(i)
                    && let Some(locator) = dim.series.coordinate_to_index.borrow().get(&coordinate)
                    && let Some(index) = locator.as_leaf()
                {
                    coords.push(index);
                } else {
                    // XXX warn
                    continue 'outer;
                }
            }

            let format = if let Some(cell_formats) = &cell_formats
                && let Some(value) = cell_formats.values.get(i)
            {
                datum_as_format(value)
            } else {
                F40_2
            };
            let mut value = datum_as_pivot_value(cell, format);

            if let Some(cell_footnotes) = &cell_footnotes
                && let Some(dv) = cell_footnotes.values.get(i)
            {
                if let Some(s) = dv.as_string() {
                    for part in s.split(',') {
                        if let Ok(index) = part.parse::<usize>()
                            && let Some(index) = index.checked_sub(1)
                            && let Some(footnote) = footnotes.get(index)
                        {
                            value.add_footnote(footnote);
                        }
                    }
                }
            }
            if let Some((series, mappings)) = &markers
                && let Some(Some(category)) = series.categories.get(i)
                && let Some(marker) = mappings.get(category)
            {
                value.add_subscript(*marker);
            }

            if !value.is_empty() {
                data.insert(coords.clone(), value);
            }
        }
        data
    }

    fn decode_styles(
        &self,
        look: &Look,
        series: &BTreeMap<&str, Series>,
        dims: &mut [Dim],
        data: &mut HashMap<Vec<usize>, Value>,
        footnotes: &pivot::Footnotes,
        styles: &HashMap<&str, &Style>,
        warn: &mut dyn FnMut(LegacyXmlWarning),
    ) {
        let has_cell_footnotes = series.contains_key("footnotes");
        for scp in self
            .facet_layout
            .children
            .iter()
            .filter_map(|child| child.set_cell_properties())
        {
            let targets = scp
                .sets
                .iter()
                .filter_map(|set| set.decode(styles))
                .collect::<Vec<_>>();

            if let Some(union_) = &scp.union_ {
                if !scp.apply_to_converse {
                    for intersect in &union_.intersects {
                        for target in &targets {
                            target.decode(
                                intersect,
                                &look,
                                &series,
                                dims,
                                data,
                                &footnotes,
                                has_cell_footnotes,
                            );
                        }
                    }
                } else {
                    // Not seen in the corpus.
                    warn(LegacyXmlWarning::UnsupportedApplyToConverse);
                }
            } else if scp.apply_to_converse {
                for target in &targets {
                    if target.target_type == TargetType::Labeling {
                        for value in data.values_mut() {
                            target.apply(
                                value,
                                &look.areas[Area::Data(RowParity::Even)],
                                footnotes,
                                has_cell_footnotes,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Faceting {
    #[serde(default, rename = "$value")]
    children: Vec<FacetingChild>,
}

impl Faceting {
    fn dimensions(&self) -> EnumMap<Axis3, Vec<&str>> {
        let mut dimensions = EnumMap::default();
        if let Some(cross) = self.cross() {
            dimensions[Axis3::X] = cross.children[0].variables();
            dimensions[Axis3::Y] = cross.children[1].variables();
        }
        dimensions[Axis3::Z] = self.layers();
        dimensions
    }
    fn cross(&self) -> Option<&Cross> {
        self.children.iter().find_map(|child| match child {
            FacetingChild::Cross(cross) => Some(cross),
            _ => None,
        })
    }

    fn layers(&self) -> Vec<&str> {
        self.children
            .iter()
            .filter_map(|child| match child {
                FacetingChild::Layer(layer) => Some(layer.variable.as_str()),
                _ => None,
            })
            .collect()
    }

    fn layer_value(&self, name: &str) -> Option<usize> {
        self.children.iter().find_map(|child| {
            if let FacetingChild::Layer(layer) = &child
                && layer.variable == name
            {
                layer.value.parse().ok()
            } else {
                None
            }
        })
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum FacetingChild {
    Cross(Cross),
    Layer(Layer),
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Cross {
    #[serde(rename = "$value")]
    children: [CrossChild; 2],
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum CrossChild {
    /// No dimensions along this axis.
    Unity,
    /// Dimensions along this axis.
    Nest(Nest),
}

impl CrossChild {
    fn variables(&self) -> Vec<&str> {
        match self {
            CrossChild::Unity => Vec::new(),
            CrossChild::Nest(nest) => nest
                .variable_references
                .iter()
                .map(|vr| vr.reference.as_str())
                .collect(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Nest {
    #[serde(rename = "variableReference")]
    variable_references: Vec<VariableReference>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Layer {
    #[serde(rename = "@variable")]
    variable: String,

    #[serde(rename = "@value")]
    value: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct FacetLayout {
    #[serde(rename = "$value")]
    children: Vec<FacetLayoutChild>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum FacetLayoutChild {
    SetCellProperties(SetCellProperties),
    FacetLevel(FacetLevel),
    #[serde(other)]
    Other,
}

impl FacetLayoutChild {
    fn set_cell_properties(&self) -> Option<&SetCellProperties> {
        match self {
            Self::SetCellProperties(scp) => Some(scp),
            _ => None,
        }
    }
    fn facet_level(&self) -> Option<&FacetLevel> {
        match self {
            Self::FacetLevel(facet_level) => Some(facet_level),
            _ => None,
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SetCellProperties {
    #[serde(rename = "@applyToConverse", default)]
    apply_to_converse: bool,

    #[serde(rename = "$value")]
    sets: Vec<Set>,

    #[serde(rename = "union")]
    union_: Option<Union>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Union {
    #[serde(default, rename = "intersect")]
    intersects: Vec<Intersect>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Intersect {
    #[serde(default, rename = "$value")]
    children: Vec<IntersectChild>,
}

impl Intersect {
    fn wheres(&self) -> impl Iterator<Item = &Where> {
        self.children.iter().filter_map(|child| match child {
            IntersectChild::Where(w) => Some(w),
            IntersectChild::Other => None,
        })
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum IntersectChild {
    Where(Where),
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Where {
    #[serde(rename = "@variable")]
    variable: String,
    #[serde(rename = "@include")]
    include: String,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum TargetType {
    Labeling,
    MajorTicks,
    Interval,
}

impl TargetType {
    fn from_id(target: &str) -> Option<Self> {
        if target == "interval" {
            Some(Self::Interval)
        } else if target == "labeling" {
            Some(Self::Labeling)
        } else if target.ends_with("majorTicks") {
            Some(Self::MajorTicks)
        } else {
            None
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum Set {
    SetFormat(SetFormat),
    SetStyle(SetStyle),
    SetFrameStyle(SetFrameStyle),
    #[serde(other)]
    Other,
}

impl Set {
    fn decode<'a>(&'a self, styles: &HashMap<&str, &'a Style>) -> Option<DecodedSet<'a>> {
        Some(DecodedSet {
            target_type: self.target_type()?,
            change: match self {
                Set::SetFormat(set_format) => Change::Format(set_format),
                Set::SetStyle(set_style) => Change::Style(set_style.style.get(styles)?),
                Set::SetFrameStyle(set_frame_style) => {
                    Change::FrameStyle(set_frame_style.style.get(styles)?)
                }
                Set::Other => return None,
            },
        })
    }

    fn target(&self) -> Option<&str> {
        match self {
            Set::SetFormat(set_format) => Some(&set_format.target),
            Set::SetStyle(set_style) => Some(&set_style.target),
            Set::SetFrameStyle(set_frame_style) => Some(&set_frame_style.target),
            Set::Other => None,
        }
    }

    fn target_type(&self) -> Option<TargetType> {
        self.target().and_then(TargetType::from_id)
    }
}

struct DecodedSet<'a> {
    target_type: TargetType,
    change: Change<'a>,
}

impl<'a> DecodedSet<'a> {
    fn decode(
        &self,
        intersect: &Intersect,
        look: &Look,
        series: &BTreeMap<&str, Series>,
        dims: &mut [Dim],
        data: &mut HashMap<Vec<usize>, Value>,
        footnotes: &pivot::Footnotes,
        has_cell_footnotes: bool,
    ) {
        match self.target_type {
            TargetType::MajorTicks => {
                // Formatting for individual row or column labels.
                for w in intersect.wheres() {
                    let Some(s) = series.get(w.variable.as_str()) else {
                        continue;
                    };
                    let Some(dim_index) = s.dimension_index.get() else {
                        continue;
                    };
                    let root = &mut dims[dim_index].root;
                    let Ok(axis) = Axis2::try_from(dims[dim_index].axis) else {
                        continue;
                    };
                    for index in w.include.split(';').filter_map(|s| s.parse::<usize>().ok()) {
                        if let Some(locator) = s.coordinate_to_index.borrow().get(&index).copied()
                            && let Some(category) = root.category_mut(locator)
                        {
                            self.apply(
                                category.name_mut(),
                                &look.areas[Area::Labels(axis)],
                                footnotes,
                                has_cell_footnotes,
                            );
                        }
                    }
                }
            }
            TargetType::Interval | TargetType::Labeling => {
                // Formatting for individual cells or groups of them
                // with some dimensions in common.
                let mut include = vec![HashSet::new(); dims.len()];
                for w in intersect.wheres() {
                    let Some(s) = series.get(w.variable.as_str()) else {
                        continue;
                    };
                    let Some(dim_index) = s.dimension_index.get() else {
                        // Group indexes may be included even though
                        // they are redundant.  Ignore them.
                        continue;
                    };
                    for index in w.include.split(';').filter_map(|s| s.parse::<usize>().ok()) {
                        if let Some(locator) = s.coordinate_to_index.borrow().get(&index).copied()
                            && let Some(leaf_index) = locator.as_leaf()
                        {
                            include[dim_index].insert(leaf_index);
                        }
                    }
                }

                // XXX This is inefficient in the common case where
                // all of the dimensions are matched.  We should use
                // a heuristic where if all of the dimensions are
                // matched and the product of n[*] is less than the
                // number of cells then iterate through all the
                // possibilities rather than all the cells.  Or even
                // only do it if there is just one possibility.
                for (indexes, value) in data {
                    let mut skip = false;
                    for (dimension, index) in indexes.iter().enumerate() {
                        if !include[dimension].is_empty() && !include[dimension].contains(index) {
                            skip = true;
                            break;
                        }
                    }
                    if !skip {
                        self.apply(
                            value,
                            &look.areas[Area::Data(RowParity::Even)],
                            footnotes,
                            has_cell_footnotes,
                        );
                    }
                }
            }
        }
    }

    fn apply(
        &self,
        value: &mut Value,
        base_style: &AreaStyle,
        footnotes: &pivot::Footnotes,
        has_cell_footnotes: bool,
    ) {
        match self.change {
            Change::Format(set_format) => Style::apply_to_value(
                value,
                Some(set_format),
                None,
                None,
                base_style,
                footnotes,
                has_cell_footnotes,
            ),
            Change::FrameStyle(style) => Style::apply_to_value(
                value,
                None,
                None,
                Some(style),
                base_style,
                footnotes,
                has_cell_footnotes,
            ),
            Change::Style(style) => {
                let (fg, bg) = if self.target_type == TargetType::Interval {
                    (None, Some(style))
                } else {
                    (Some(style), None)
                };
                Style::apply_to_value(
                    value,
                    None,
                    fg,
                    bg,
                    base_style,
                    footnotes,
                    has_cell_footnotes,
                )
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum Change<'a> {
    Format(&'a SetFormat),
    Style(&'a Style),
    FrameStyle(&'a Style),
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SetStyle {
    #[serde(rename = "@target")]
    target: String,

    #[serde(rename = "@style")]
    style: Ref<Style>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SetFrameStyle {
    #[serde(rename = "@target")]
    target: String,

    #[serde(rename = "@style")]
    style: Ref<Style>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SetFormat {
    #[serde(rename = "@target")]
    target: String,

    #[serde(rename = "$value")]
    child: Option<SetFormatChild>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum SetFormatChild {
    Format(Format),
    NumberFormat(NumberFormat),
    StringFormat(Vec<StringFormat>),
    DateTimeFormat(DateTimeFormat),
    ElapsedTimeFormat(ElapsedTimeFormat),
}

impl SetFormatChild {
    fn decode_format(&self) -> Option<format::Format> {
        match self {
            SetFormatChild::Format(format) => Some(format.decode()),
            SetFormatChild::NumberFormat(format) => {
                Some(SignificantNumberFormat::from(format).decode())
            }
            SetFormatChild::StringFormat(_) => None,
            SetFormatChild::DateTimeFormat(format) => Some(format.decode()),
            SetFormatChild::ElapsedTimeFormat(format) => Some(format.decode()),
        }
    }
    fn affixes(&self) -> &[Affix] {
        match self {
            SetFormatChild::Format(format) => &format.affixes,
            SetFormatChild::NumberFormat(number_format) => &number_format.affixes,
            SetFormatChild::StringFormat(string_formats) => {
                string_formats.first().map_or(&[], |first| &first.affixes)
            }
            SetFormatChild::DateTimeFormat(date_time_format) => &date_time_format.affixes,
            SetFormatChild::ElapsedTimeFormat(elapsed_time_format) => &elapsed_time_format.affixes,
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Interval {
    labeling: Labeling,
    footnotes: Option<Footnotes>,
}

impl Interval {
    fn footnotes(&self, superscript: bool) -> Option<&Footnotes> {
        if let Some(footnotes) = &self.footnotes
            && footnotes.superscript == superscript
        {
            Some(footnotes)
        } else {
            self.labeling
                .children
                .iter()
                .flat_map(|child| child.as_footnotes())
                .find(|child| child.superscript == superscript)
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Labeling {
    #[serde(rename = "$value", default)]
    children: Vec<LabelingChild>,
}

impl Labeling {
    fn decode_format_map<'a>(&self, series: &'a BTreeMap<&str, Series>) -> Option<&'a Series> {
        let mut cell_format = None;
        for child in &self.children {
            if let LabelingChild::Formatting(formatting) = child {
                cell_format = series.get(formatting.variable.as_str());
            }
        }
        cell_format
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum LabelingChild {
    Formatting(Formatting),
    Footnotes(Footnotes),
    #[serde(other)]
    Other,
}

impl LabelingChild {
    fn as_footnotes(&self) -> Option<&Footnotes> {
        match self {
            Self::Footnotes(footnotes) => Some(footnotes),
            _ => None,
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Formatting {
    #[serde(rename = "@variable")]
    variable: String,
}

#[derive(Clone, Debug, Default)]
struct Footnote {
    content: String,
    marker: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Footnotes {
    #[serde(rename = "@variable")]
    variable: String,

    #[serde(default, rename = "@superscript")]
    superscript: bool,

    #[serde(default, rename = "footnoteMapping")]
    mappings: Vec<FootnoteMapping>,
}

impl Footnotes {
    fn decode(&self, dst: &mut BTreeMap<usize, Footnote>) {
        for f in &self.mappings {
            let index = f.defines_reference.unwrap_or(f.from).get() - 1;
            dst.entry(index).or_default().content = f.to.clone();
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct FootnoteMapping {
    #[serde(rename = "@definesReference")]
    defines_reference: Option<NonZeroUsize>,

    #[serde(rename = "@from")]
    from: NonZeroUsize,

    #[serde(rename = "@to")]
    to: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct FacetLevel {
    #[serde(rename = "@level")]
    level: usize,

    axis: Axis,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Axis {
    label: Option<Label>,
    major_ticks: MajorTicks,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct MajorTicks {
    #[serde(rename = "@style")]
    style: Ref<Style>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Label {
    #[serde(rename = "@style")]
    style: Ref<Style>,

    #[serde(rename = "@textFrameStyle")]
    text_frame_style: Option<Ref<Style>>,

    #[serde(rename = "@purpose")]
    purpose: Option<Purpose>,

    #[serde(rename = "$value")]
    child: LabelChild,
}

impl Label {
    fn text(&self) -> &[Text] {
        match &self.child {
            LabelChild::Text(texts) => texts.as_slice(),
            LabelChild::Other => &[],
        }
    }
}
#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "camelCase")]
enum Purpose {
    Title,
    SubTitle,
    SubSubTitle,
    Layer,
    Footnote,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum LabelChild {
    Text(Vec<Text>),
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Text {
    /// If this is present, then `text` defines the content or the marker for a
    /// footnote.
    #[serde(rename = "@usesReference")]
    uses_reference: Option<NonZeroUsize>,

    /// If this is present, then
    #[serde(rename = "@definesReference")]
    defines_reference: Option<NonZeroUsize>,

    #[serde(default, rename = "$text")]
    text: String,
}

enum DecodedText<'a> {
    /// Defines footnote `index` content or marker as `text`.
    FootnoteDefinition { index: usize, text: &'a str },

    /// Adds a reference to footnote `index`.
    FootnoteReference { index: usize },

    /// Text content.
    Text { text: &'a str },
}

impl Text {
    fn decode(&self) -> DecodedText<'_> {
        if let Some(uses_reference) = self.uses_reference {
            DecodedText::FootnoteDefinition {
                index: uses_reference.get() - 1,
                text: &self.text,
            }
        } else if let Some(defines_reference) = self.defines_reference {
            DecodedText::FootnoteReference {
                index: defines_reference.get() - 1,
            }
        } else {
            DecodedText::Text { text: &self.text }
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LabelFrame {
    label: Option<Label>,
}

impl LabelFrame {
    fn decode_label(labels: &[&Label], footnotes: &pivot::Footnotes) -> Option<Value> {
        if !labels.is_empty() {
            let mut s = String::new();
            let mut f = Vec::new();
            for label in labels {
                for text in label.text() {
                    match text.decode() {
                        DecodedText::FootnoteReference { index } => {
                            if let Some(footnote) = footnotes.get(index) {
                                f.push(footnote)
                            }
                        }
                        DecodedText::Text { text } => s += text,
                        _ => (),
                    }
                }
            }
            Some(Value::new_user_text(s).with_footnotes(f))
        } else {
            None
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Container {
    #[serde(rename = "labelFrame")]
    #[serde(default)]
    label_frames: Vec<LabelFrame>,
}
fn decode_dimension<'a>(
    variables: &[(&'a Series, usize)],
    axes: &HashMap<usize, &Axis>,
    styles: &HashMap<&str, &Style>,
    a: Axis3,
    footnotes: &pivot::Footnotes,
    dims: &mut Vec<Dim<'a>>,
) {
    let base_level = variables[0].1;
    let (show_label, dim_cell, dim_font, dim_label) = if let Ok(a) = Axis2::try_from(a)
        && let Some(axis) = axes.get(&(base_level + variables.len()))
        && let Some(label) = &axis.label
    {
        let mut dimension_style = AreaStyle::default_for_area(Area::Labels(a));
        let style = label.style.get(&styles);
        let fg = style;
        let bg = label.text_frame_style.as_ref().and_then(|r| r.get(styles));
        (
            style.is_some_and(|s| s.visible.unwrap_or(true)),
            Style::decode_cell_style(fg, &mut dimension_style.cell_style)
                .then_some(dimension_style.cell_style),
            Style::decode_font_style(fg, bg, &mut dimension_style.font_style)
                .then_some(dimension_style.font_style),
            LabelFrame::decode_label(&[label], footnotes),
        )
    } else {
        (false, None, None, None)
    };

    let hide_all_labels = if let Some(axis) = axes.get(&base_level)
        && let Some(style) = axis.major_ticks.style.get(styles)
        && style.visible == Some(false)
    {
        true
    } else {
        false
    };

    let variables = variables
        .into_iter()
        .map(|(series, _level)| *series)
        .collect::<Vec<_>>();

    #[derive(Clone, Debug)]
    struct CatBuilder {
        /// The category we've built so far.
        category: Category,

        /// The index in the series of one example of this category.
        index: usize,

        /// The range of leaf indexes covered by `category`.
        ///
        /// If `category` is a leaf, the range has a length of 1.
        /// If `category` is a group, the length is at least 1.
        leaves: Range<usize>,

        /// How to find this category in its dimension.
        location: CategoryLocator,
    }

    // Make leaf categories.
    let mut map = BTreeMap::new();
    for (index, category) in variables[0].categories.iter().enumerate() {
        if let Some(coordinate) = category {
            map.entry(*coordinate).or_insert(index);
        }
    }
    let mut coordinate_to_index = BTreeMap::new();
    let mut cats = Vec::<CatBuilder>::new();
    for (coordinate, index) in map {
        let value = &variables[0].values[index];
        coordinate_to_index.insert(coordinate, CategoryLocator::new_leaf(cats.len()));
        cats.push(CatBuilder {
            category: Category::from(Leaf::new(variables[0].new_name(value, footnotes))),
            index,
            leaves: cats.len()..cats.len() + 1,
            location: CategoryLocator::new_leaf(cats.len()),
        });
    }
    *variables[0].coordinate_to_index.borrow_mut() = coordinate_to_index;

    // Now group them, in one pass per grouping variable, innermost first.
    for variable in &variables[1..] {
        let mut coordinate_to_index = BTreeMap::new();
        let mut next_cats = Vec::with_capacity(cats.len());
        let mut start = 0;
        for end in 1..=cats.len() {
            let dv1 = &variable.values[cats[start].index];
            if end >= cats.len() || &variable.values[cats[end].index] != dv1 {
                if !dv1.is_spaces() {
                    let name = variable.new_name(dv1, footnotes);
                    let next_cat = CatBuilder {
                        category: Group::new(name)
                            .with_multiple(cats[start..end].iter().map(|c| c.category.clone()))
                            .into(),
                        index: cats[start].index,
                        leaves: cats[start].leaves.start..cats[end - 1].leaves.end,
                        location: cats[start].location.parent(),
                    };
                    coordinate_to_index.insert(
                        variable.categories[cats[start].index].unwrap(),
                        next_cat.location,
                    );
                    next_cats.push(next_cat);
                } else {
                    for cat in &cats[start..end] {
                        next_cats.push(cat.clone());
                    }
                };
                start = end;
            }
        }
        *variable.coordinate_to_index.borrow_mut() = coordinate_to_index;
        cats = next_cats;
    }

    let mut dimension_label = if let Some(dim_label) = dim_label {
        dim_label
    } else if let Some(label) = &variables[0].label {
        Value::new_user_text(label)
    } else {
        Value::new_empty()
    };
    if let Some(dim_cell) = dim_cell {
        dimension_label.set_cell_style(dim_cell);
    }
    if let Some(dim_font) = dim_font {
        dimension_label.set_font_style(dim_font);
    }

    let root = Group::new(dimension_label)
        .with_multiple(cats.into_iter().map(|cb| cb.category))
        .with_show_label(show_label);

    for variable in &variables {
        variable.dimension_index.set(Some(dims.len()));
    }
    dims.push(Dim {
        axis: a,
        hide_all_labels,
        root,
        series: variables[0],
    });
}

fn decode_axis_dimensions<'a, 'b>(
    variables: impl IntoIterator<Item = &'a str>,
    series: &'b BTreeMap<&str, Series>,
    axes: &HashMap<usize, &Axis>,
    styles: &HashMap<&str, &Style>,
    a: Axis3,
    footnotes: &pivot::Footnotes,
    level_ofs: usize,
    dims: &mut Vec<Dim<'b>>,
) -> Vec<&'b Series> {
    let variables = variables
        .into_iter()
        .zip(level_ofs..)
        .map(|(variable_name, level)| {
            series
                .get(variable_name)
                .filter(|s| !s.values.is_empty())
                .map(|s| (s, level))
        })
        .collect::<Vec<_>>();
    let mut dim_vars = Vec::new();
    let mut categorical_vars = Vec::new();
    for var in variables {
        if let Some((var, level)) = var {
            dim_vars.push((var, level));
        } else if !dim_vars.is_empty() {
            categorical_vars.push(dim_vars[0].0);
            decode_dimension(&dim_vars, axes, styles, a, footnotes, dims);
            dim_vars.clear();
        }
    }
    if !dim_vars.is_empty() {
        categorical_vars.push(&dim_vars[0].0);
        decode_dimension(&dim_vars, axes, styles, a, footnotes, dims);
    }
    categorical_vars
}

struct Dim<'a> {
    axis: Axis3,
    root: pivot::Group,
    hide_all_labels: bool,
    series: &'a Series,
}
