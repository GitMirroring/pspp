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

//! HTML parsing and formatting.
//!
//! SPV files contain text in a simple subset of HTML.  [Markup] represents
//! parsed text in this form suitable for a single [Value], whereas [Document]
//! can contain multiple paragraphs of markup, each paragraph represented by a
//! [Block].
#![warn(dead_code)]
use std::{
    borrow::{Borrow, Cow},
    fmt::{Display, Write as _},
    io::{Cursor, Write},
    mem::{discriminant, take},
    str::FromStr,
};

use hashbrown::HashMap;
use html_parser::{Dom, Element, Node};
use pango::{AttrColor, AttrInt, AttrList, AttrSize, AttrString, IsAttribute};
use quick_xml::{
    Writer as XmlWriter,
    escape::resolve_html5_entity,
    events::{BytesRef, BytesText, Event},
};
use serde::{Deserialize, Deserializer, Serialize, ser::SerializeMap};

use crate::output::pivot::{
    look::{CellStyle, Color, FontStyle, HorzAlign},
    value::Value,
};

fn lowercase<'a>(s: &'a str) -> Cow<'a, str> {
    if s.chars().any(|c| c.is_ascii_uppercase()) {
        Cow::from(s.to_ascii_lowercase())
    } else {
        Cow::from(s)
    }
}

/// Inline styled text.
#[derive(Clone, Debug, PartialEq)]
pub enum Markup {
    /// A sequence.
    Seq(
        /// The sequence.
        Vec<Markup>,
    ),
    /// A text string.
    Text(
        /// The text.
        String,
    ),
    /// A substitution variable.
    Variable(
        /// The variable.
        Variable,
    ),
    /// Styled text.
    Style {
        /// The style to apply to the contents of `child`.
        style: Style,
        /// The styled child markup.
        child: Box<Markup>,
    },
}

/// A substitution variable within [Markup].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Variable {
    /// `&[Date]`
    Date,
    /// `&[Time]`
    Time,
    /// `&[HeadN]`
    Head(
        /// `N`.
        u8,
    ),
    /// `&[PageTitle]`.
    PageTitle,
    /// `&[Page]`.
    Page,
}

/// Unknown variable error returned by [Variable::from_str].
#[derive(Copy, Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Unknown variable")]
pub struct UnknownVariable;

impl FromStr for Variable {
    type Err = UnknownVariable;

    /// Parses `Date` into [Self::Date], and so on.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Date" => Ok(Self::Date),
            "Time" => Ok(Self::Time),
            "PageTitle" => Ok(Self::PageTitle),
            "Page" => Ok(Self::Page),
            _ => {
                if let Some(suffix) = s.strip_prefix("Head")
                    && let Ok(number) = suffix.parse()
                    && number >= 1
                {
                    Ok(Self::Head(number))
                } else {
                    Err(UnknownVariable)
                }
            }
        }
    }
}

impl Variable {
    fn as_str(&self) -> Cow<'static, str> {
        match self {
            Variable::Date => Cow::from("Date"),
            Variable::Time => Cow::from("Time"),
            Variable::Head(index) => Cow::from(format!("Head{index}")),
            Variable::PageTitle => Cow::from("PageTitle"),
            Variable::Page => Cow::from("Page"),
        }
    }
}

impl Display for Variable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Default for Markup {
    fn default() -> Self {
        Self::Seq(Vec::new())
    }
}

impl Serialize for Markup {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Markup::Seq(inner) => serializer.collect_seq(inner),
            Markup::Text(string) => serializer.serialize_str(string.as_str()),
            Markup::Variable(name) => serializer.serialize_newtype_struct("Variable", name),
            Markup::Style { style, child } => {
                let (mut style, mut child) = (style, child);
                let mut styles = HashMap::new();
                loop {
                    styles.insert(discriminant(style), style);
                    match &**child {
                        Markup::Style {
                            style: inner,
                            child: inner_child,
                        } => {
                            style = inner;
                            child = inner_child;
                        }
                        _ => break,
                    }
                }
                let mut map = serializer.serialize_map(Some(styles.len() + 1))?;
                for style in styles.into_values() {
                    match style {
                        Style::Bold => map.serialize_entry("bool", &true),
                        Style::Italic => map.serialize_entry("italic", &true),
                        Style::Underline => map.serialize_entry("underline", &true),
                        Style::Strike => map.serialize_entry("strike", &true),
                        Style::Emphasis => map.serialize_entry("em", &true),
                        Style::Strong => map.serialize_entry("strong", &true),
                        Style::Face(name) => map.serialize_entry("font", name),
                        Style::Color(color) => map.serialize_entry("color", color),
                        Style::Size(size) => map.serialize_entry("size", size),
                    }?;
                }
                map.serialize_entry("content", child)?;
                map.end()
            }
        }
    }
}

impl Display for Markup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn inner(this: &Markup, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match this {
                Markup::Seq(seq) => {
                    for markup in seq {
                        inner(markup, f)?;
                    }
                    Ok(())
                }
                Markup::Text(string) => f.write_str(string.as_str()),
                Markup::Variable(name) => write!(f, "&[{name}]"),
                Markup::Style { child, .. } => inner(child, f),
            }
        }
        inner(self, f)
    }
}

impl Markup {
    /// Returns true if this markup contains no text.
    pub fn is_empty(&self) -> bool {
        match self {
            Markup::Seq(seq) => seq.iter().all(|markup| markup.is_empty()),
            Markup::Text(s) => s.is_empty(),
            Markup::Variable(_) => false,
            Markup::Style { style: _, child } => child.is_empty(),
        }
    }
    /// Returns true if this is a [Markup::Style].
    pub fn is_style(&self) -> bool {
        matches!(self, Markup::Style { .. })
    }
    /// If this is [Markup::Style], returns its contents, and otherwise `None`.
    pub fn into_style(self) -> Option<(Style, Markup)> {
        match self {
            Markup::Style { style, child } => Some((style, *child)),
            _ => None,
        }
    }
    /// Returns true if this is a [Markup::Text].
    pub fn is_text(&self) -> bool {
        matches!(self, Markup::Text(_))
    }
    /// For [Markup::Text], returns the text, and otherwise `None`.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Markup::Text(text) => Some(text.as_str()),
            _ => None,
        }
    }
    /// If this is [Markup::Text], returns its contents, and otherwise `None`.
    pub fn into_text(self) -> Option<String> {
        match self {
            Markup::Text(text) => Some(text),
            _ => None,
        }
    }
    fn write_html<X>(&self, writer: &mut XmlWriter<X>) -> std::io::Result<()>
    where
        X: Write,
    {
        match self {
            Markup::Seq(children) => {
                for child in children {
                    child.write_html(writer)?;
                }
            }
            Markup::Text(text) => writer.write_event(Event::Text(BytesText::new(text.as_str())))?,
            Markup::Variable(name) => {
                writer.write_event(Event::Text(BytesText::new(&format!("&[{name}]"))))?
            }
            Markup::Style { style, child } => {
                let mut elements = Vec::new();
                let mut attributes = Vec::new();
                fn add_style(
                    style: &Style,
                    elements: &mut Vec<&'static str>,
                    attributes: &mut Vec<(&'static str, String)>,
                ) {
                    match style {
                        Style::Bold => elements.push("b"),
                        Style::Italic => elements.push("i"),
                        Style::Underline => elements.push("u"),
                        Style::Strike => elements.push("strike"),
                        Style::Emphasis => elements.push("em"),
                        Style::Strong => elements.push("strong"),
                        Style::Face(face) => attributes.push(("face", face.clone())),
                        Style::Color(color) => {
                            attributes.push(("color", color.display_css().to_string()))
                        }
                        Style::Size(points) => {
                            attributes.push(("size", format!("{}pt", *points / 0.75)))
                        }
                    }
                }

                add_style(style, &mut elements, &mut attributes);
                let mut next = &**child;
                while let Markup::Style { style, child } = next {
                    add_style(style, &mut elements, &mut attributes);
                    next = &**child;
                }

                elements.sort();
                attributes.sort();
                next.write_styles(writer, &elements, &attributes)?;
            }
        }
        Ok(())
    }

    fn write_styles<X>(
        &self,
        writer: &mut XmlWriter<X>,
        elements: &[&str],
        attributes: &[(&str, String)],
    ) -> std::io::Result<()>
    where
        X: Write,
    {
        if !attributes.is_empty() {
            writer
                .create_element("font")
                .with_attributes(
                    attributes
                        .into_iter()
                        .map(|(name, value)| (*name, Cow::from(value))),
                )
                .write_inner_content(|w| self.write_styles(w, elements, &[]))?;
        } else if let Some((element, rest)) = elements.split_first() {
            writer
                .create_element(*element)
                .write_inner_content(|w| self.write_styles(w, rest, attributes))?;
        } else {
            self.write_html(writer)?;
        }
        Ok(())
    }

    /// Returns this markup converted into XHTML.  The returned string contains
    /// a single `<html>...</html>` element.
    ///
    /// Substitution variables in the markup are converted back into their
    /// source forms as `&[PageTitle]`, etc.
    pub fn to_html(&self) -> String {
        let mut writer = XmlWriter::new(Cursor::new(Vec::new()));
        writer
            .create_element("html")
            .write_inner_content(|w| self.write_html(w))
            .expect("writing to a Vec can't fail");
        String::from_utf8(writer.into_inner().into_inner())
            .expect("XmlWriter should only output UTF-8")
    }

    /// Returns this markup as text and attributes suitable for passing as the
    /// argument to [pango::Layout::set_text] and
    /// [pango::Layout::set_attributes], respectively.
    ///
    /// Calls `expand` to obtain expansions for variables in the markup.
    pub fn to_pango<'a, F>(&self, expand: F) -> (String, AttrList)
    where
        F: Fn(Variable) -> Option<Cow<'a, str>>,
    {
        let mut s = String::new();
        let mut attrs = AttrList::new();
        self.to_pango_inner(&expand, &mut s, &mut attrs);
        (s, attrs)
    }

    fn to_pango_inner<'a, F>(&self, expand: &F, s: &mut String, attrs: &mut AttrList)
    where
        F: Fn(Variable) -> Option<Cow<'a, str>>,
    {
        match self {
            Markup::Seq(seq) => {
                for child in seq {
                    child.to_pango_inner(expand, s, attrs);
                }
            }
            Markup::Text(string) => s.push_str(&string),
            Markup::Variable(variable) => match expand(*variable) {
                Some(value) => s.push_str(&*value),
                None => write!(s, "&[{variable}]").unwrap(),
            },
            Markup::Style { style, child } => {
                let start_index = s.len();
                child.to_pango_inner(expand, s, attrs);
                let end_index = s.len();

                let mut attr = match style {
                    Style::Bold | Style::Strong => {
                        AttrInt::new_weight(pango::Weight::Bold).upcast()
                    }
                    Style::Italic | Style::Emphasis => {
                        AttrInt::new_style(pango::Style::Italic).upcast()
                    }
                    Style::Underline => AttrInt::new_underline(pango::Underline::Single).upcast(),
                    Style::Strike => AttrInt::new_strikethrough(true).upcast(),
                    Style::Face(face) => AttrString::new_family(&face).upcast(),
                    Style::Color(color) => {
                        let (r, g, b) = color.into_rgb16();
                        AttrColor::new_foreground(r, g, b).upcast()
                    }
                    Style::Size(points) => AttrSize::new((points * 1024.0) as i32).upcast(),
                };
                attr.set_start_index(start_index as u32);
                attr.set_end_index(end_index as u32);
                attrs.insert(attr);
            }
        }
    }

    fn parse_variables(&self) -> Option<Vec<Markup>> {
        let Some(mut s) = self.as_text() else {
            return None;
        };
        let mut results = Vec::new();
        let mut offset = 0;
        while let Some(start) = s[offset..].find("&[").map(|pos| pos + offset)
            && let Some(end) = s[start..].find("]").map(|pos| pos + start)
        {
            if let Ok(variable) = Variable::from_str(&s[start + 2..end]) {
                if start > 0 {
                    results.push(Markup::Text(s[..start].into()));
                }
                results.push(Markup::Variable(variable));
                s = &s[end + 1..];
                offset = 0;
            } else {
                offset = end + 1;
            }
        }
        if results.is_empty() {
            None
        } else {
            if !s.is_empty() {
                results.push(Markup::Text(s.into()));
            }
            Some(results)
        }
    }

    fn append(&mut self, other: &mut Self) {
        let mut a = take(self).into_seq();
        let mut b = take(other).into_seq();
        if let Some(Markup::Text(head)) = a.last_mut()
            && let Some(Markup::Text(tail)) = b.first()
        {
            head.push_str(&tail);
            a.extend(b.drain(1..));
        } else {
            a.append(&mut b);
        }
        *self = Self::from_seq(a);
    }

    fn into_seq(self) -> Vec<Markup> {
        if self.is_empty() {
            Vec::new()
        } else if let Markup::Seq(markups) = self {
            markups
        } else {
            vec![self]
        }
    }

    fn from_seq(seq: Vec<Self>) -> Self {
        if seq.is_empty() {
            Self::default()
        } else {
            match <[Self; 1]>::try_from(seq) {
                Ok([singleton]) => singleton,
                Err(multiple) => Self::Seq(multiple),
            }
        }
    }
}

/// A block of styled text.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Block {
    /// Contents.
    pub markup: Markup,

    /// Horizontal alignment.
    pub horz_align: HorzAlign,
}

impl Default for Block {
    fn default() -> Self {
        Self {
            markup: Markup::default(),
            horz_align: HorzAlign::Left,
        }
    }
}

impl Block {
    fn new(mut markup: Markup, horz_align: HorzAlign, css: &[Style]) -> Self {
        for style in css {
            apply_style(&mut markup, style.clone());
        }
        Self { markup, horz_align }
    }

    /// Returns a [Value] with this `Block`'s contents.
    pub fn into_value(self) -> Value {
        let mut font_style = FontStyle::default().with_size(10);
        let cell_style = CellStyle::default().with_horz_align(Some(self.horz_align));
        let mut strike = false;
        let mut markup = self.markup;
        let mut markup = loop {
            if let Markup::Style { style, child } = markup {
                match style {
                    Style::Bold => font_style.bold = true,
                    Style::Italic => font_style.italic = true,
                    Style::Underline => font_style.underline = true,
                    Style::Strike => strike = true,
                    Style::Emphasis => font_style.italic = true,
                    Style::Strong => font_style.bold = true,
                    Style::Face(face) => font_style.font = face,
                    Style::Color(color) => font_style.fg = color,
                    Style::Size(points) => font_style.size = points as i32,
                };
                markup = *child;
            } else {
                break markup;
            }
        };
        if strike {
            apply_style(&mut markup, Style::Strike);
        }
        match markup {
            Markup::Text(text) => Value::new_user_text(text),
            markup => Value::new_markup(markup),
        }
        .with_font_style(font_style)
        .with_cell_style(cell_style)
    }
}

/// Blocks of styled text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document(
    /// The blocks.
    pub Vec<Block>,
);

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Document::from_html(&String::deserialize(deserializer)?))
    }
}

impl Serialize for Document {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_html().serialize(serializer)
    }
}

impl Document {
    /// Returns true if this document contains no [Block]s.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Parses HTML `input` into a `Document`.  If `input` is not valid HTML,
    /// then it is treated as plain text instead.
    pub fn from_html(input: &str) -> Self {
        match Dom::parse(&format!("<!doctype html>{input}")) {
            Ok(dom) => Self(parse_dom(&dom)),
            Err(_) if !input.is_empty() => Self(vec![Block {
                markup: Markup::Text(input.into()),
                horz_align: HorzAlign::Left,
            }]),
            Err(_) => Self::default(),
        }
    }

    /// Returns the document converted to a [Value].  If the document has
    /// multiple [Block]s, then they are concatenated with new-lines in between.
    pub fn into_value(self) -> Value {
        let mut iter = self.0.into_iter();
        let mut block = iter.next().unwrap_or_default();
        for mut additional in iter {
            block.markup.append(&mut Markup::Text(String::from("\n")));
            block.markup.append(&mut additional.markup);
        }
        block.into_value()
    }

    /// Returns the document converted to XHTML, except that the result will not
    /// be a single `<html>...</html>` element but instead the contents for such
    /// an element.
    pub fn to_html(&self) -> String {
        let mut writer = XmlWriter::new(Cursor::new(Vec::new()));
        writer
            .create_element("html")
            .write_inner_content(|w| {
                for block in &self.0 {
                    w.create_element("p")
                        .with_attribute(("align", block.horz_align.as_str().unwrap_or("right")))
                        .write_inner_content(|w| block.markup.write_html(w))?;
                }
                Ok(())
            })
            .expect("writing to a Vec can't fail");

        // Return the result with `<html>` and `</html>` stripped off.
        str::from_utf8(&writer.into_inner().into_inner())
            .expect("XmlWriter should only output UTF-8")
            .strip_prefix("<html>")
            .expect("<html> should always be present")
            .strip_suffix("</html>")
            .expect("</html> should always be present")
            .into()
    }

    /// Returns the document converted to a series of [Value]s.
    pub fn to_values(&self) -> Vec<Value> {
        self.0
            .iter()
            .map(|block| block.clone().into_value())
            .collect()
    }
}

/// A text style.
///
/// Used in [Markup::Style].
#[derive(Clone, Debug, PartialEq)]
pub enum Style {
    /// **Bold**.
    Bold,

    /// *Italic*.
    Italic,

    /// __Underline__.
    Underline,

    /// ~~Strikethrough~~.
    Strike,

    /// <em>Emphasis</em>.
    Emphasis,

    /// <strong>Strong</strong>.
    Strong,

    /// Sets the typeface.
    Face(
        /// The typeface name.
        String,
    ),

    /// Font color.
    Color(
        /// The color
        Color,
    ),

    /// Font size.
    Size(
        /// In 1/72" units.
        f64,
    ),
}

fn node_as_element<'a>(node: &'a Node, name: &str) -> Option<&'a Element> {
    if let Node::Element(element) = node
        && element.name.eq_ignore_ascii_case(name)
    {
        Some(element)
    } else {
        None
    }
}

fn node_is_element(node: &Node, name: &str) -> bool {
    node_as_element(node, name).is_some()
}

/// Returns the horizontal alignment for the `<p>` element in `p`.
fn horz_align_from_p(p: &Element) -> Option<HorzAlign> {
    if let Some(Some(s)) = p.attributes.get("align")
        && let Ok(align) = HorzAlign::from_str(s)
    {
        Some(align)
    } else if let Some(Some(s)) = p.attributes.get("style")
        && let Some(align) = HorzAlign::from_css(s)
    {
        Some(align)
    } else {
        None
    }
}

fn apply_style(markup: &mut Markup, style: Style) {
    let child = take(markup);
    *markup = Markup::Style {
        style,
        child: Box::new(child),
    };
}

fn parse_dom(dom: &Dom) -> Vec<Block> {
    // Get the top-level elements, descending into an `html` element if
    // there is one.
    let roots = if dom.children.len() == 1
        && let Some(first) = dom.children.first()
        && let Some(html) = node_as_element(first, "html")
    {
        &html.children
    } else {
        &dom.children
    };

    // If there's a `head` element, parse it for CSS and then skip past it.
    let mut head_styles = Vec::new();
    let mut default_horz_align = HorzAlign::Left;
    let roots = if let Some((first, rest)) = roots.split_first()
        && let Some(head) = node_as_element(first, "head")
    {
        if let Some(style) = find_element(&head.children, "style") {
            let mut text = String::new();
            get_element_text(style, &mut text);
            head_styles = Style::parse_css(&text);
            if let Some(horz_align) = HorzAlign::from_css(&text) {
                default_horz_align = horz_align;
            }
        }
        rest
    } else {
        roots
    };

    // If only a `body` element is left, descend into it.
    let body = if roots.len() == 1
        && let Some(first) = roots.first()
        && let Some(body) = node_as_element(first, "body")
    {
        &body.children
    } else {
        roots
    };

    let mut blocks = Vec::new();

    let mut start = 0;
    while start < body.len() {
        let (end, align) = if let Some(p) = node_as_element(&body[start], "p") {
            (
                start + 1,
                horz_align_from_p(p).unwrap_or(default_horz_align),
            )
        } else {
            let mut end = start + 1;
            while end < body.len() && !node_is_element(&body[end], "p") {
                end += 1;
            }
            (end, default_horz_align)
        };
        blocks.push(Block::new(
            parse_nodes(&body[start..end]),
            align,
            &head_styles,
        ));
        start = end;
    }

    blocks
}

fn unescape(mut input: &str) -> Cow<'_, str> {
    let mut output = String::new();
    while let Some(amp) = input.find('&') {
        if amp > 0 {
            output.push_str(&input[..amp]);
        }
        input = &input[amp + 1..];
        if let Some(semi) = input.find([';', '&']) {
            let entity = &input[..semi];
            let rest = &input[semi + 1..];
            if let Ok(Some(c)) = BytesRef::new(entity).resolve_char_ref() {
                output.push(c);
                input = rest;
            } else if let Some(resolution) = resolve_html5_entity(entity) {
                output.push_str(resolution);
                input = rest;
            } else {
                output.push('&');
            }
        } else {
            output.push('&');
        }
    }
    if output.is_empty() {
        Cow::from(input)
    } else {
        output.push_str(input);
        Cow::from(output)
    }
}

fn parse_nodes(nodes: &[Node]) -> Markup {
    // Appends `markup` to `dst`, merging text at the end of `dst` with text
    // in `markup`.
    fn add_markup(dst: &mut Vec<Markup>, markup: Markup) {
        if let Markup::Text(suffix) = &markup
            && let Some(Markup::Text(last)) = dst.last_mut()
        {
            last.push_str(&suffix);
        } else {
            dst.push(markup);
        }

        if let Some(last) = dst.last()
            && let Some(mut expansion) = last.parse_variables()
        {
            dst.pop();
            dst.append(&mut expansion);
        }
    }

    let mut retval = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        match node {
            Node::Comment(_) => (),
            Node::Text(text) => {
                let text = if i == 0 {
                    text.trim_start()
                } else {
                    text.as_str()
                };
                let text = if i == nodes.len() - 1 {
                    text.trim_end()
                } else {
                    text
                };
                add_markup(&mut retval, Markup::Text(unescape(&text).into_owned()));
            }
            // SPSS often starts paragraphs with an initial `<BR>` that it
            // ignores, but it does honor `<br>`.  So weird.
            Node::Element(br)
                if br.name.eq_ignore_ascii_case("br") && (br.name == "br" || i != 0) =>
            {
                add_markup(&mut retval, Markup::Text('\n'.into()));
            }
            Node::Element(element) => {
                let mut inner = parse_nodes(&element.children);
                if inner.is_empty() {
                    continue;
                }

                let style = match lowercase(&element.name).borrow() {
                    "b" => Some(Style::Bold),
                    "i" => Some(Style::Italic),
                    "u" => Some(Style::Underline),
                    "s" | "strike" => Some(Style::Strike),
                    "strong" => Some(Style::Strong),
                    "em" => Some(Style::Emphasis),
                    "font" => {
                        if let Some(Some(face)) = element.attributes.get("face") {
                            apply_style(&mut inner, Style::Face(face.clone()));
                        }
                        if let Some(Some(color)) = element.attributes.get("color")
                            && let Ok(color) = Color::from_str(&color)
                        {
                            apply_style(&mut inner, Style::Color(color));
                        }
                        if let Some(Some(html_size)) = element.attributes.get("size")
                            && let Ok(html_size) = usize::from_str(&html_size)
                            && let Some(index) = html_size.checked_sub(1)
                            && let Some(points) =
                                [6.0, 7.5, 9.0, 10.5, 13.5, 18.0, 27.0].get(index).copied()
                        {
                            apply_style(&mut inner, Style::Size(points * 0.75));
                        }
                        None
                    }
                    _ => None,
                };
                match style {
                    None => match inner {
                        Markup::Seq(seq) => {
                            for markup in seq {
                                add_markup(&mut retval, markup);
                            }
                        }
                        _ => add_markup(&mut retval, inner),
                    },
                    Some(style) => retval.push(Markup::Style {
                        style,
                        child: Box::new(inner),
                    }),
                }
            }
        }
    }
    if retval.len() == 1 {
        retval.into_iter().next().unwrap()
    } else {
        Markup::Seq(retval)
    }
}

fn find_element<'a>(elements: &'a [Node], name: &str) -> Option<&'a Element> {
    for element in elements {
        if let Node::Element(element) = element
            && element.name == name
        {
            return Some(element);
        }
    }
    None
}

fn parse_entity(s: &str) -> (char, &str) {
    static ENTITIES: [(&str, char); 6] = [
        ("amp;", '&'),
        ("lt;", '<'),
        ("gt;", '>'),
        ("apos;", '\''),
        ("quot;", '"'),
        ("nbsp;", '\u{00a0}'),
    ];
    for (name, ch) in ENTITIES {
        if let Some(rest) = s.strip_prefix(name) {
            return (ch, rest);
        }
    }
    ('&', s)
}

fn get_node_text(node: &Node, text: &mut String) {
    match node {
        Node::Text(string) => {
            let mut s = string.as_str();
            while !s.is_empty() {
                let amp = s.find('&').unwrap_or(s.len());
                let (head, rest) = s.split_at(amp);
                text.push_str(head);
                if rest.is_empty() {
                    break;
                }
                let ch;
                (ch, s) = parse_entity(&s[1..]);
                text.push(ch);
            }
        }
        Node::Element(element) => get_element_text(element, text),
        Node::Comment(_) => (),
    }
}

fn get_element_text(element: &Element, text: &mut String) {
    for child in &element.children {
        get_node_text(child, text);
    }
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, str::FromStr};

    use crate::spv::read::html::{self, Document, Markup, Variable};

    #[test]
    fn variable() {
        assert_eq!(Variable::from_str("Head1").unwrap(), Variable::Head(1));
        assert_eq!(Variable::from_str("Page").unwrap(), Variable::Page);
        assert_eq!(Variable::from_str("Date").unwrap(), Variable::Date);
        assert_eq!(Variable::Head(1).to_string(), "Head1");
        assert_eq!(Variable::Page.to_string(), "Page");
        assert_eq!(Variable::Date.to_string(), "Date");
    }

    #[test]
    fn parse_variables() {
        assert_eq!(Markup::Text("asdf".into()).parse_variables(), None);
        assert_eq!(Markup::Text("&[asdf]".into()).parse_variables(), None);
        assert_eq!(
            Markup::Text("&[Page]".into()).parse_variables(),
            Some(vec![Markup::Variable(Variable::Page)])
        );
        assert_eq!(
            Markup::Text("xyzzy &[Invalid] &[Page] &[Invalid2] quux".into()).parse_variables(),
            Some(vec![
                Markup::Text("xyzzy &[Invalid] ".into()),
                Markup::Variable(Variable::Page),
                Markup::Text(" &[Invalid2] quux".into()),
            ])
        );
    }

    /// Example from the documentation.
    #[test]
    fn example1() {
        let text = r##"<xml>&lt;html xmlns="http://www.w3.org/1999/xhtml" lang="en">
  &lt;head>

  &lt;/head>
  &lt;body>
    &lt;p>
      plain&amp;#160;&lt;font color="#000000" size="3" face="Monospaced">&lt;b>bold&lt;/b>&lt;/font>&amp;#160;&lt;font color="#000000" size="3" face="Monospaced">&lt;i>italic&lt;/i>&amp;#160;&lt;strike>strikeout&lt;/strike>&lt;/font>
    &lt;/p>
  &lt;/body>
&lt;/html>
</xml>"##;
        let content = quick_xml::de::from_str::<String>(text).unwrap();
        assert_eq!(
            Document::from_html(&content).to_html(),
            r##"<p align="left">plain <font color="#000000" face="Monospaced" size="9pt"><b>bold</b></font> <font color="#000000" face="Monospaced" size="9pt"><i>italic</i> <strike>strikeout</strike></font></p>"##
        );
    }

    /// Another example from the documentation.
    #[test]
    fn example2() {
        let text = r##"<xml>&lt;html xmlns="http://www.w3.org/1999/xhtml" lang="en">
  &lt;head>

  &lt;/head>
  &lt;body>
    &lt;p>left&lt;/p>
    &lt;p align="center">&lt;font color="#000000" size="5" face="Monospaced">center&amp;#160;large&lt;/font>&lt;/p>
    &lt;p align="right">&lt;font color="#000000" size="3" face="Monospaced">&lt;b>&lt;i>right&lt;/i>&lt;/b>&lt;/font>&lt;/p>
  &lt;/body>
&lt;/html></xml>
"##;
        let content = quick_xml::de::from_str::<String>(text).unwrap();
        assert_eq!(
            Document::from_html(&content).to_html(),
            r##"<p align="left">left</p><p align="center"><font color="#000000" face="Monospaced" size="13.5pt">center large</font></p><p align="right"><font color="#000000" face="Monospaced" size="9pt"><b><i>right</i></b></font></p>"##
        );
    }

    /// From the corpus, demonstrating how SPSS sometimes writes `&` instead of `&amp;`.
    #[test]
    fn invalid_entities() {
        let text = r##"<xml>&lt;head>&lt;style type="text/css">p{color:0;font-family:Monospaced;font-size:14pt;font-style:normal;font-weight:normal;text-decoration:none}&lt;/style>&lt;/head>&lt;BR>Stem-and-Leaf&amp;nbsp;Plot&amp;nbsp;for&lt;br>&lt;/br>Foobar=&amp;nbsp;K(+)&lt;br>&lt;/br>&lt;br>&lt;/br>&amp;nbsp;Frequency&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;Stem&amp;nbsp;&amp;&amp;nbsp;&amp;nbsp;Leaf&lt;br>&lt;/br>&lt;br>&lt;/br>&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;1.00&amp;nbsp;Extremes&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;(=&amp;lt;4)&lt;br>&lt;/br>&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;4.00&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;0&amp;nbsp;.&amp;nbsp;&amp;nbsp;6666&lt;br>&lt;/br>&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;1.00&amp;nbsp;Extremes&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;(&amp;gt;=8)&lt;br>&lt;/br>&lt;br>&lt;/br>&amp;nbsp;Stem&amp;nbsp;width:&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;10.00&lt;br>&lt;/br>&amp;nbsp;Each&amp;nbsp;leaf:&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;&amp;nbsp;1&amp;nbsp;case(s)&lt;br>&lt;/br>&lt;br>&lt;/br>&lt;br>&lt;/br></xml>
"##;
        let content = quick_xml::de::from_str::<String>(text).unwrap();
        assert_eq!(
            Document::from_html(&content).to_html(),
            r##"<p align="left"><font face="Monospaced" size="14pt">Stem-and-Leaf Plot for
Foobar= K(+)

 Frequency    Stem &amp;  Leaf

     1.00 Extremes    (=&lt;4)
     4.00        0 .  6666
     1.00 Extremes    (&gt;=8)

 Stem width:     10.00
 Each leaf:       1 case(s)


</font></p>"##
        );
    }

    /// From the corpus (also included in the documentation).
    #[test]
    fn header1() {
        let text = r##"<xml>&lt;html xmlns="http://xml.spss.com/spss/viewer/viewer-tree">
  &lt;head>

  &lt;/head>
  &lt;body>
    &lt;p style="text-align:center; margin-top: 0">
      &amp;[PageTitle]
    &lt;/p>
  &lt;/body>
&lt;/html></xml>"##;
        let content = quick_xml::de::from_str::<String>(text).unwrap();
        assert_eq!(
            Document::from_html(&content).to_html(),
            r##"<p align="center">&amp;[PageTitle]</p>"##
        );
    }

    /// From the corpus (also included in the documentation).
    #[test]
    fn footer1() {
        let text = r##"<xml>&lt;html xmlns="http://xml.spss.com/spss/viewer/viewer-tree">
  &lt;head>

  &lt;/head>
  &lt;body>
    &lt;p style="text-align:right; margin-top: 0">
      Page &amp;[Page]
    &lt;/p>
  &lt;/body>
&lt;/html></xml>"##;
        let content = quick_xml::de::from_str::<String>(text).unwrap();
        assert_eq!(
            Document::from_html(&content).to_html(),
            r##"<p align="right">Page &amp;[Page]</p>"##
        );
    }

    /// From the corpus (also included in the documentation).
    #[test]
    fn header2() {
        let text = r##"<xml>&lt;html xmlns="http://www.w3.org/1999/xhtml" lang="en">
  &lt;head>
          &lt;style type="text/css">
                  p { font-family: sans-serif;
                       font-size: 10pt; text-align: center;
                       font-weight: normal;
                       color: #000000;
                       }
          &lt;/style>
  &lt;/head>
  &lt;body>
          &lt;p>&amp;amp;[PageTitle]&lt;/p>
  &lt;/body>
&lt;/html></xml>"##;
        let content = quick_xml::de::from_str::<String>(text).unwrap();
        let document = Document::from_html(&content);
        assert_eq!(
            document.to_html(),
            r##"<p align="center"><font color="#000000" face="sans-serif" size="10pt">&amp;[PageTitle]</font></p>"##
        );
        assert_eq!(
            document.0[0]
                .markup
                .to_pango(
                    |name| (name == html::Variable::PageTitle).then_some(Cow::from("The title"))
                )
                .0,
            "The title"
        );
    }

    /// From the corpus (also included in the documentation).
    #[test]
    fn footer2() {
        let text = r##"<xml>&lt;html xmlns="http://www.w3.org/1999/xhtml" lang="en">
  &lt;head>
          &lt;style type="text/css">
                  p { font-family: sans-serif;
                       font-size: 10pt; text-align: right;
                       font-weight: normal;
                       color: #000000;
                       }
          &lt;/style>
  &lt;/head>
  &lt;body>
          &lt;p>Page &amp;amp;[Page]&lt;/p>
  &lt;/body>
&lt;/html>
</xml>"##;
        let content = quick_xml::de::from_str::<String>(text).unwrap();
        let html = Document::from_html(&content);
        assert_eq!(
            html.to_html(),
            r##"<p align="right"><font color="#000000" face="sans-serif" size="10pt">Page &amp;[Page]</font></p>"##
        );
    }

    /// From the corpus, anonymized.
    ///
    /// This tests the unusual treatment of `<BR>` at the start of text (`<BR>`
    /// is ignored at the start, but `<br>` is not).
    #[test]
    fn breaks() {
        let text = r##"<xml>&lt;head>&lt;style type="text/css">p{color:0;font-family:Monospaced;font-size:13pt;font-style:normal;font-weight:normal;text-decoration:none}&lt;/style>&lt;/head>&lt;BR>USE ALL.&lt;BR>COMPUTE filter_$=(group = 1).&lt;BR>VARIABLE LABEL filter_$ 'group = 1 (FILTER)'.&lt;BR>VALUE LABELS filter_$ 0 'Not Selected' 1 'Selected'.&lt;BR>FORMAT filter_$ (f1.0).&lt;BR>FILTER BY filter_$.&lt;BR>EXECUTE.&lt;BR>NPAR TEST&lt;BR>  /WILCOXON=x WITH y&lt;BR>   z w (PAIRED)&lt;BR>  /MISSING ANALYSIS.</xml>"##;
        let content = quick_xml::de::from_str::<String>(text).unwrap();
        let html = Document::from_html(&content);
        let s = html.into_value().display(()).to_string();
        assert_eq!(
            s,
            r##"USE ALL.
COMPUTE filter_$=(group = 1).
VARIABLE LABEL filter_$ 'group = 1 (FILTER)'.
VALUE LABELS filter_$ 0 'Not Selected' 1 'Selected'.
FORMAT filter_$ (f1.0).
FILTER BY filter_$.
EXECUTE.
NPAR TEST
  /WILCOXON=x WITH y
   z w (PAIRED)
  /MISSING ANALYSIS."##
        );
    }

    /// From the corpus, anonymized.
    ///
    /// This tests treatment of multiple paragraphs in a context where we
    /// usually expect only one.
    #[test]
    fn multiparagraph_value() {
        let text = r##"<xml>&lt;html>
  

  &lt;head>

  &lt;/head>
  &lt;body>
    &lt;p style="margin-top: 0">
      H0:There is no association between X and Y
    &lt;/p>
    &lt;p style="margin-top: 0">
      H1:There is association between X and Y
    &lt;/p>
  &lt;/body>
&lt;/html>
</xml>"##;
        let content = quick_xml::de::from_str::<String>(text).unwrap();
        let html = Document::from_html(&content);
        let s = html.into_value().display(()).to_string();
        assert_eq!(
            s,
            "H0:There is no association between X and Y
H1:There is association between X and Y"
        );
    }

    /// Checks that the `escape-html` feature is enabled in [quick_xml], since
    /// we need that to resolve `&nbsp;` and other HTML entities.
    #[test]
    fn html_escapes() {
        let html = Document::from_html("&nbsp;");
        assert_eq!(html.to_html(), "<p align=\"left\">\u{a0}</p>")
    }
}
