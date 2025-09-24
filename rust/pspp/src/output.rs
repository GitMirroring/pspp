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

#![allow(dead_code)]
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt::Display,
    iter::once,
    mem::take,
    str::FromStr,
    sync::{Arc, OnceLock},
};

use anyhow::anyhow;
use bit_vec::BitVec;
use cairo::ImageSurface;
use clap::{ArgAction, ArgMatches, Args, FromArgMatches, value_parser};
use enum_map::EnumMap;
use enumset::{EnumSet, EnumSetType};
use itertools::Itertools;
use pivot::PivotTable;
use serde::Serialize;

use crate::{
    message::{Diagnostic, Severity},
    output::pivot::{
        Axis3, Dimension, Group,
        look::{BorderStyle, Look},
        value::Value,
    },
};

pub mod drivers;
pub mod page;
pub mod pivot;
pub mod render;
pub mod table;

/// A single output item.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    /// The localized label for the item that appears in the outline pane in the
    /// output viewer and in PDF outlines.  This is `None` if no label has been
    /// explicitly set.
    pub label: Option<String>,

    /// A locale-invariant identifier for the command that produced the output,
    /// which may be `None` if unknown or if a command did not produce this
    /// output.
    pub command_name: Option<String>,

    /// For a heading item, this is true if the heading's subtree should
    /// be expanded in an outline view, false otherwise.
    ///
    /// For other kinds of output items, this is true to show the item's
    /// content, false to hide it.  The item's label is always shown in an
    /// outline view.
    pub show: bool,

    /// Whether the item should start at the top of a page.
    pub page_break_before: bool,

    /// Item details.
    pub details: Details,

    /// If the item was read from an SPV file, this is additional information
    /// related to that file.
    #[serde(skip_serializing)]
    pub spv_info: Option<Box<SpvInfo>>,
}

pub trait Itemlike {
    fn label(&self) -> Cow<'_, str>;
    fn command_name(&self) -> Option<&str>;
    fn subtype(&self) -> Option<String>;
    fn is_shown(&self) -> Option<bool>;
    fn page_break_before(&self) -> bool;
    fn spv_info(&self) -> Option<Cow<'_, SpvInfo>>;
    fn iter_in_order(&self) -> ItemRefIterator<'_, Self>
    where
        Self: Sized;
    fn children(&self) -> &[Arc<Self>];
    fn children_mut(&mut self) -> Option<&mut Vec<Arc<Self>>>;
    fn kind(&self) -> ItemKind;
    fn is_heading(&self) -> bool {
        self.kind() == ItemKind::Heading
    }
    fn is_expanded(&self) -> Option<bool>;
    fn is_table(&self) -> bool {
        self.kind() == ItemKind::Table
    }
    fn class(&self) -> Class {
        match self.kind() {
            ItemKind::Graph => Class::Graphs,
            ItemKind::Image => Class::Other,
            ItemKind::Heading => Class::OutlineHeaders,
            ItemKind::Message(severity) => match severity {
                Severity::Note => Class::Notes,
                Severity::Error | Severity::Warning => Class::Warnings,
            },
            ItemKind::Table => match self.label().as_ref() {
                "Warnings" => Class::Warnings,
                "Notes" => Class::Notes,
                _ => Class::Tables,
            },
            ItemKind::Text => match self.label().as_ref() {
                "Title" => Class::Headings,
                "Log" => Class::Logs,
                "Page Title" => Class::PageTitle,
                _ => Class::Texts,
            },
            ItemKind::Model => Class::Models,
            ItemKind::Tree => Class::Trees,
        }
    }
}

impl Itemlike for Item {
    fn label(&self) -> Cow<'_, str> {
        match &self.label {
            Some(label) => Cow::from(label.clone()),
            None => self.details.label(),
        }
    }

    fn command_name(&self) -> Option<&str> {
        self.command_name.as_deref()
    }

    fn subtype(&self) -> Option<String> {
        self.details
            .as_table()
            .map(|table| table.subtype().display(table).to_string())
    }

    /// Should the item be shown?
    ///
    /// This returns None for headings because their contents are always shown
    /// (although headings can be collapsed in an outline view).
    fn is_shown(&self) -> Option<bool> {
        if !self.details.is_heading() {
            Some(self.show)
        } else {
            None
        }
    }

    fn is_expanded(&self) -> Option<bool> {
        if self.details.is_heading() {
            Some(self.show)
        } else {
            None
        }
    }

    fn page_break_before(&self) -> bool {
        self.page_break_before
    }

    fn spv_info(&self) -> Option<Cow<'_, SpvInfo>> {
        self.spv_info
            .as_ref()
            .map(|spv_info| Cow::Borrowed(&**spv_info))
    }

    fn iter_in_order(&self) -> ItemRefIterator<'_, Item> {
        ItemRefIterator::new(self)
    }

    fn children(&self) -> &[Arc<Self>] {
        self.details.children()
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Arc<Item>>> {
        self.details.children_mut()
    }

    fn kind(&self) -> ItemKind {
        self.details.kind()
    }
}

impl Item {
    pub fn new(details: impl Into<Details>) -> Self {
        let details = details.into();
        Self {
            page_break_before: false,
            label: None,
            command_name: details.command_name().cloned(),
            show: true,
            details,
            spv_info: None,
        }
    }

    pub fn subtype(&self) -> Option<String> {
        self.details
            .as_table()
            .map(|table| table.subtype().display(table).to_string())
    }

    pub fn with_show(self, show: bool) -> Self {
        Self { show, ..self }
    }

    pub fn with_label(self, label: impl Into<String>) -> Self {
        Self {
            label: Some(label.into()),
            ..self
        }
    }

    pub fn with_command_name(self, command_name: Option<String>) -> Self {
        Self {
            command_name,
            ..self
        }
    }

    pub fn with_some_command_name(self, command_name: impl Into<String>) -> Self {
        self.with_command_name(Some(command_name.into()))
    }

    pub fn with_spv_info(self, spv_info: SpvInfo) -> Self {
        Self {
            spv_info: Some(Box::new(spv_info)),
            ..self
        }
    }

    pub fn with_page_break_before(self, page_break_before: bool) -> Self {
        Self {
            page_break_before,
            ..self
        }
    }

    pub fn cursor(self: Arc<Self>) -> ItemCursor<Self> {
        ItemCursor::new(self)
    }
}

impl<T> From<T> for Item
where
    T: Into<Details>,
{
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<A> FromIterator<A> for Item
where
    A: Into<Arc<Item>>,
{
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = A>,
    {
        iter.into_iter().collect::<Details>().into_item()
    }
}

impl PivotTable {
    pub fn into_item(self) -> Item {
        Details::Table(Box::new(self)).into_item()
    }
}

/// A group of output items.
///
/// The name "heading" is used in SPV files.  There is only a visible
/// heading if the output items inside the heading include a [Text] item.
/// The grouping itself is only visible in the outline pane in a viewer
/// window.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Heading(pub Vec<Arc<Item>>);

impl Heading {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, item: impl Into<Arc<Item>>) -> Self {
        self.0.push(item.into());
        self
    }

    pub fn into_item(self) -> Item {
        Details::Heading(self).into_item()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Graph,
    Image,
    Heading,
    Message(Severity),
    Table,
    Text,
    Model,
    Tree,
}

impl ItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemKind::Graph => "graph",
            ItemKind::Image => "image",
            ItemKind::Heading => "heading",
            ItemKind::Message(_) => "message",
            ItemKind::Table => "table",
            ItemKind::Text => "text",
            ItemKind::Model => "model",
            ItemKind::Tree => "tree",
        }
    }
}

impl Display for ItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, Debug, Serialize)]
pub enum Details {
    Graph,
    Image(#[serde(skip_serializing)] ImageSurface),
    Heading(Heading),
    Message(Box<Diagnostic>),
    Table(Box<PivotTable>),
    Text(Box<Text>),
}

impl Details {
    pub fn into_item(self) -> Item {
        Item::new(self)
    }

    pub fn children(&self) -> &[Arc<Item>] {
        match self {
            Self::Heading(heading) => heading.0.as_slice(),
            _ => &[],
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<Arc<Item>>> {
        match self {
            Self::Heading(heading) => Some(&mut heading.0),
            _ => None,
        }
    }

    pub fn as_message(&self) -> Option<&Diagnostic> {
        match self {
            Self::Message(diagnostic) => Some(diagnostic),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<&PivotTable> {
        match self {
            Self::Table(table) => Some(table),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&Text> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_image(&self) -> Option<&ImageSurface> {
        match self {
            Self::Image(image_surface) => Some(image_surface),
            _ => None,
        }
    }

    pub fn command_name(&self) -> Option<&String> {
        match self {
            Details::Graph
            | Details::Image(_)
            | Details::Heading(_)
            | Details::Message(_)
            | Details::Text(_) => None,
            Details::Table(pivot_table) => pivot_table.metadata.command_c.as_ref(),
        }
    }

    pub fn label(&self) -> Cow<'static, str> {
        match self {
            Details::Graph => Cow::from("Graph"),
            Details::Image(_) => Cow::from("Image"),
            Details::Heading(_) => Cow::from("Group"),
            Details::Message(diagnostic) => Cow::from(diagnostic.severity.as_title_str()),
            Details::Table(pivot_table) => Cow::from(pivot_table.label()),
            Details::Text(text) => Cow::from(text.type_.as_str()),
        }
    }

    pub fn is_heading(&self) -> bool {
        matches!(self, Self::Heading(_))
    }

    pub fn is_message(&self) -> bool {
        matches!(self, Self::Message(_))
    }

    pub fn is_table(&self) -> bool {
        matches!(self, Self::Table(_))
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    pub fn kind(&self) -> ItemKind {
        match self {
            Details::Graph => ItemKind::Graph,
            Details::Image(_) => ItemKind::Image,
            Details::Heading(_) => ItemKind::Heading,
            Details::Message(diagnostic) => ItemKind::Message(diagnostic.severity),
            Details::Table(_) => ItemKind::Table,
            Details::Text(_) => ItemKind::Text,
        }
    }
}

impl<A> FromIterator<A> for Details
where
    A: Into<Arc<Item>>,
{
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = A>,
    {
        Self::Heading(Heading(
            iter.into_iter().map(|value| value.into()).collect(),
        ))
    }
}

impl From<Diagnostic> for Details {
    fn from(value: Diagnostic) -> Self {
        Self::Message(Box::new(value))
    }
}

impl From<Box<Diagnostic>> for Details {
    fn from(value: Box<Diagnostic>) -> Self {
        Self::Message(value)
    }
}

impl From<PivotTable> for Details {
    fn from(value: PivotTable) -> Self {
        Self::Table(Box::new(value))
    }
}

impl From<Box<PivotTable>> for Details {
    fn from(value: Box<PivotTable>) -> Self {
        Self::Table(value)
    }
}

impl From<Text> for Details {
    fn from(value: Text) -> Self {
        Self::Text(Box::new(value))
    }
}

impl From<Box<Text>> for Details {
    fn from(value: Box<Text>) -> Self {
        Self::Text(value)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Graph;

impl Graph {
    pub fn into_item(self) -> Item {
        Details::Graph.into_item()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Text {
    pub type_: TextType,

    pub content: Value,
}

impl Text {
    pub fn new(type_: TextType, content: impl Into<Value>) -> Self {
        Self {
            type_,
            content: content.into(),
        }
    }
    pub fn new_log(content: impl Into<Value>) -> Self {
        Self::new(TextType::Log, content)
    }

    pub fn into_item(self) -> Item {
        Details::Text(Box::new(self)).into_item()
    }
}

fn text_item_table_look() -> Arc<Look> {
    static LOOK: OnceLock<Arc<Look>> = OnceLock::new();
    LOOK.get_or_init(|| {
        Arc::new({
            let mut look = Look::default().with_borders(EnumMap::from_fn(|_| BorderStyle::none()));
            for style in look.areas.values_mut() {
                style.cell_style.margins = EnumMap::from_fn(|_| [0, 0]);
            }
            look
        })
    })
    .clone()
}

impl From<Text> for PivotTable {
    fn from(value: Text) -> Self {
        let dimension =
            Dimension::new(Group::new(Value::new_text("Text")).with(Value::new_user_text("null")))
                .with_all_labels_hidden();
        PivotTable::new([(Axis3::Y, dimension)])
            .with_look(text_item_table_look())
            .with_data([(&[0], value.content)])
            .with_subtype(Value::new_user_text("Text"))
    }
}

impl From<&Diagnostic> for Text {
    fn from(value: &Diagnostic) -> Self {
        Self::new_log(value.to_string())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextType {
    /// `TITLE` and `SUBTITLE` commands.
    PageTitle,

    /// Title,
    Title,

    /// Syntax printback logging.
    Syntax,

    /// Other logging.
    Log,
}

impl TextType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TextType::PageTitle => "Page Title",
            TextType::Title => "Title",
            TextType::Syntax => "Log",
            TextType::Log => "Log",
        }
    }

    pub fn as_xml_str(&self) -> &'static str {
        match self {
            TextType::PageTitle => "page-title",
            TextType::Title => "title",
            TextType::Syntax | TextType::Log => "log",
        }
    }
}

pub struct ItemRefIterator<'a, T> {
    next: Option<&'a T>,
    stack: Vec<(&'a T, usize)>,
}

impl<'a, T> ItemRefIterator<'a, T> {
    pub fn without_hidden(self) -> impl Iterator<Item = &'a T>
    where
        T: Itemlike,
    {
        self.filter(|item| item.is_shown().unwrap_or(true))
    }

    pub fn new(start: &'a T) -> Self {
        Self {
            next: Some(start),
            stack: Vec::new(),
        }
    }
}

impl<'a, T> Iterator for ItemRefIterator<'a, T>
where
    T: Itemlike,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.next.take()?;
        if let Some(first_child) = cur.children().first() {
            self.next = Some(first_child.as_ref());
            self.stack.push((cur, 1));
        } else {
            while let Some((item, index)) = self.stack.pop() {
                if let Some(child) = item.children().get(index) {
                    self.next = Some(child.as_ref());
                    self.stack.push((item, index + 1));
                    return Some(cur);
                }
            }
        }
        Some(cur)
    }
}

pub struct ItemCursor<T> {
    cur: Option<Arc<T>>,
    stack: Vec<(Arc<T>, usize)>,
    include_hidden: bool,
}

impl<T> ItemCursor<T>
where
    T: Itemlike,
{
    pub fn new(start: Arc<T>) -> Self {
        Self {
            cur: start.as_ref().is_shown().unwrap_or(true).then_some(start),
            stack: Vec::new(),
            include_hidden: false,
        }
    }

    pub fn with_hidden(start: Arc<T>) -> Self {
        Self {
            cur: Some(start),
            stack: Vec::new(),
            include_hidden: true,
        }
    }

    pub fn cur(&self) -> Option<&Arc<T>> {
        self.cur.as_ref()
    }

    pub fn next(&mut self) {
        fn inner<T>(this: &mut ItemCursor<T>)
        where
            T: Itemlike,
        {
            let Some(cur) = this.cur.take() else {
                return;
            };
            if let Some(first_child) = cur.as_ref().children().first() {
                this.cur = Some(first_child.clone());
                this.stack.push((cur, 1));
            } else {
                while let Some((item, index)) = this.stack.pop() {
                    if let Some(child) = item.as_ref().children().get(index) {
                        this.cur = Some(child.clone());
                        this.stack.push((item, index + 1));
                        return;
                    }
                }
            }
        }

        inner(self);
        while let Some(cur) = &self.cur
            && !cur.as_ref().is_shown().unwrap_or(true)
        {
            inner(self);
        }
    }

    // Returns the label for the heading with the given `level` in the stack
    // above the current item.  Level 0 is the top level.  Levels without a
    // label are skipped.
    pub fn heading(&self, level: usize) -> Option<Cow<'_, str>> {
        self.stack
            .iter()
            .map(|(item, _index)| item.as_ref().label())
            .filter(|label| !label.is_empty())
            .nth(level)
    }
}

/// Information for output items that were read from an SPV file.
///
/// This is for debugging and troubleshooting purposes.
#[derive(Clone, Debug)]
pub struct SpvInfo {
    /// True if there was an error reading the output item (e.g. because of
    /// corruption or because PSPP doesn't understand the format).
    pub error: bool,

    /// Name of structure member in ZIP file.
    pub structure_member: String,

    /// Additional members based on item type.
    pub members: Option<SpvMembers>,
}

impl SpvInfo {
    pub fn new(structure_member: impl Into<String>) -> Self {
        Self {
            error: false,
            structure_member: structure_member.into(),
            members: None,
        }
    }

    pub fn with_members(self, members: Option<SpvMembers>) -> Self {
        Self { members, ..self }
    }

    pub fn with_error(self, error: bool) -> Self {
        Self { error, ..self }
    }

    pub fn member_names(&self) -> Vec<&str> {
        let mut member_names = vec![self.structure_member.as_str()];
        if let Some(members) = &self.members {
            member_names.extend(members.iter());
        }
        member_names
    }
}

/// Identifies ZIP file members for one kind of output item in an SPV file.
#[derive(Clone, Debug)]
pub enum SpvMembers {
    /// Light table detail members.
    LightTable(
        /// `.bin` member name.
        String,
    ),
    /// Legacy table detail members.
    LegacyTable {
        /// `.xml` member name.
        xml: String,
        /// `.bin` member name.
        binary: String,
    },
    /// Image members.
    Image(
        /// `.png` file.
        String,
    ),
    /// Graph members.
    Graph {
        /// Data member name.
        data: Option<String>,
        /// XML member name.
        xml: String,
        /// CVS member name.
        csv: Option<String>,
    },
}

impl SpvMembers {
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        let (a, b, c) = match self {
            SpvMembers::LightTable(a) => (Some(a), None, None),
            SpvMembers::LegacyTable { xml, binary } => (Some(xml), Some(binary), None),
            SpvMembers::Image(a) => (Some(a), None, None),
            SpvMembers::Graph { data, xml, csv } => (data.as_ref(), Some(xml), csv.as_ref()),
        };
        once(a)
            .flatten()
            .chain(once(b).flatten())
            .chain(once(c).flatten())
            .map(|s| s.as_str())
    }

    /// Returns the name of the binary data member, if any.  Only graphs and
    /// legacy tables have a binary data member.
    pub fn bin_member(&self) -> Option<&str> {
        match self {
            SpvMembers::LegacyTable { binary, .. } => Some(&binary),
            SpvMembers::Graph { data: binary, .. } => binary.as_ref().map(|s| s.as_str()),
            _ => None,
        }
    }
}

/// Classifications for output items.  These only roughly correspond to the
/// output item types; for example, "warnings" are a subset of text items.
#[derive(Debug, EnumSetType)]
pub enum Class {
    Graphs,
    Headings,
    Logs,
    Models,
    Tables,
    Texts,
    Trees,
    Warnings,
    OutlineHeaders,
    PageTitle,
    Notes,
    Unknown,
    Other,
}

impl FromStr for Class {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "graphs" | "charts" => Ok(Self::Graphs),
            "headings" => Ok(Self::Headings),
            "logs" => Ok(Self::Logs),
            "models" => Ok(Self::Models),
            "tables" => Ok(Self::Tables),
            "texts" => Ok(Self::Texts),
            "trees" => Ok(Self::Trees),
            "warnings" => Ok(Self::Warnings),
            "outlineheaders" => Ok(Self::OutlineHeaders),
            "pagetitle" => Ok(Self::PageTitle),
            "notes" => Ok(Self::Notes),
            "unknown" => Ok(Self::Unknown),
            "other" => Ok(Self::Other),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// - `None`: Include all objects.
    /// - `Some(true)`: Include only visible objects.
    /// - `Some(false)`: Include only hidden objects.
    pub visible: Option<bool>,

    /// - `None`: Include all objects.
    /// - `Some(false)`: Include only objects with no error on loading.
    /// - `Some(true)`: Include only objects with an error on loading.
    pub error: Option<bool>,

    /// Classes to include.
    pub classes: EnumSet<Class>,

    /// Command names to match.
    pub commands: StringMatch,

    /// Subtypes to match.
    pub subtypes: StringMatch,

    /// Labels to match.
    pub labels: StringMatch,

    /// Include objects under commands with the given 1-based indexes.  Without
    /// any indexes, include all objects.
    pub nth_commands: Vec<usize>,

    /// Include the objects with the given 1-based indexes within each of the
    /// commands that are included.  Indexes are 1-based.  Index 0 represents
    /// the last instance in a command.
    pub instances: Vec<usize>,

    /// Include only XML and binary member names that match.  Without any member
    /// names, include all objects.
    pub members: Vec<String>,
}

impl Selection {
    pub fn parse_nth_commands(s: &str) -> Result<Vec<usize>, anyhow::Error> {
        s.split(',')
            .map(|s| match s.trim().parse::<usize>() {
                Ok(n) if n > 0 => Ok(n),
                Ok(_) => Err(anyhow!("--nth-commmands values must be positive")),
                Err(error) => Err(error.into()),
            })
            .collect()
    }

    pub fn parse_instances(s: &str) -> Result<Vec<usize>, anyhow::Error> {
        s.split(',')
            .map(|s| {
                let s = s.trim();
                if s == "last" {
                    Ok(0)
                } else {
                    match s.parse::<usize>() {
                        Ok(n) if n > 0 => Ok(n),
                        Ok(_) => Err(anyhow!("--instances values must be positive")),
                        Err(error) => Err(error.into()),
                    }
                }
            })
            .collect()
    }

    pub fn parse_classes(s: &str) -> Result<EnumSet<Class>, anyhow::Error> {
        if s.is_empty() {
            return Ok(EnumSet::all());
        }
        let (s, invert) = match s.strip_prefix('^') {
            Some(rest) => (rest, true),
            None => (s, false),
        };
        let mut classes = EnumSet::empty();
        for name in s.split(',') {
            if name == "all" {
                classes = EnumSet::all();
            } else {
                classes.insert(
                    name.trim()
                        .parse()
                        .map_err(|_| anyhow!("unknown output class `{name}`"))?,
                );
            }
        }
        if invert {
            classes = !classes;
        }
        Ok(classes)
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            visible: Some(true),
            error: None,
            classes: EnumSet::all(),
            commands: Default::default(),
            subtypes: Default::default(),
            labels: Default::default(),
            nth_commands: Default::default(),
            members: Default::default(),
            instances: Default::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StringMatch {
    /// Include any string that matches any of the patterns.
    Include(Vec<String>),
    /// Include only the strings that do not match any of the patterns.
    Exclude(Vec<String>),
}

impl Default for StringMatch {
    fn default() -> Self {
        // Include everything, by excluding nothing.
        Self::Exclude(Vec::new())
    }
}

impl StringMatch {
    pub fn is_default(&self) -> bool {
        if let Self::Exclude(strings) = self
            && strings.is_empty()
        {
            true
        } else {
            false
        }
    }
    pub fn matches(&self, s: &str) -> bool {
        fn inner(items: &[String], s: &str) -> bool {
            items.iter().any(|item| match item.strip_suffix('*') {
                Some(prefix) => s.starts_with(prefix),
                None => s == item,
            })
        }

        match self {
            StringMatch::Include(items) => inner(items, s),
            StringMatch::Exclude(items) => !inner(items, s),
        }
    }
}

/// Can't fail.
#[derive(Debug, thiserror::Error)]
pub enum Infallible {}

impl FromStr for StringMatch {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = s.strip_prefix("^") {
            Ok(Self::Exclude(rest.split(",").map_into().collect()))
        } else {
            Ok(Self::Include(s.split(",").map_into().collect()))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Criteria(pub Vec<Selection>);

impl Criteria {
    /// Returns output items that are a subset of `input` that match the
    /// criteria.
    pub fn apply<T>(&self, input: Vec<T>) -> Vec<Arc<T>>
    where
        T: Itemlike + Clone,
    {
        fn take_children<T: Itemlike>(item: &T) -> Vec<&T> {
            item.children().iter().map(|item| item.as_ref()).collect()
        }
        fn flatten_children<'a, T: Itemlike>(
            children: Vec<&'a T>,
            depth: usize,
            items: &mut Vec<(&'a T, usize)>,
        ) {
            for child in children {
                flatten(child, depth, items);
            }
        }
        fn flatten<'a, T: Itemlike>(item: &'a T, depth: usize, items: &mut Vec<(&'a T, usize)>) {
            let children = take_children(item);
            items.push((item, depth));
            flatten_children(children, depth + 1, items);
        }

        fn select_matches<T: Itemlike>(
            items: &[(&T, usize)],
            selection: &Selection,
            include: &mut BitVec,
        ) {
            let mut instance_within_command = 0;
            let mut last_instance = None;
            let mut command_item = None;
            let mut command_command_item = None;
            let mut nth_command = 0;
            for (index, (item, depth)) in items.into_iter().enumerate() {
                if *depth == 0 {
                    command_item = Some(index);
                    if let Some(last_instance) = last_instance.take() {
                        include.set(last_instance, true);
                    }
                    instance_within_command = 0;
                }
                if !selection.classes.contains(item.class()) {
                    continue;
                }
                if let Some(visible) = selection.visible
                    && let Some(shown) = item.is_shown()
                    && visible != shown
                {
                    continue;
                }
                if let Some(error) = selection.error
                    && error != item.spv_info().map_or(false, |spv_info| spv_info.error)
                {
                    continue;
                }
                if !selection
                    .commands
                    .matches(item.command_name().unwrap_or(""))
                {
                    continue;
                }
                if !selection.nth_commands.is_empty() {
                    if command_item != command_command_item {
                        command_command_item = command_command_item;
                        nth_command += 1;
                    }
                    if !selection.nth_commands.contains(&nth_command) {
                        continue;
                    }
                }
                if !selection.subtypes.is_default() {
                    if !item.is_table() {
                        continue;
                    };
                    let subtype = item.subtype();
                    if !selection.subtypes.matches(subtype.as_deref().unwrap_or("")) {
                        continue;
                    }
                }
                if !selection.labels.matches(&item.label()) {
                    continue;
                }
                if !selection.members.is_empty() {
                    let Some(spv_info) = item.spv_info() else {
                        continue;
                    };
                    let member_names = spv_info.member_names();
                    if !selection
                        .members
                        .iter()
                        .any(|name| member_names.contains(&name.as_str()))
                    {
                        continue;
                    }
                }
                if !selection.instances.is_empty() {
                    if *depth == 0 {
                        continue;
                    }
                    instance_within_command += 1;
                    if !selection.instances.contains(&instance_within_command) {
                        if selection.instances.contains(&0) {
                            last_instance = Some(index);
                        }
                        continue;
                    }
                }

                include.set(index, true);
            }
        }
        fn unflatten_items<T: Itemlike + Clone>(
            items: Vec<Arc<T>>,
            include: &mut bit_vec::Iter,
            out: &mut Vec<Arc<T>>,
        ) {
            for item in items {
                unflatten_item(Arc::unwrap_or_clone(item), include, out);
            }
        }
        fn unflatten_item<T: Itemlike + Clone>(
            mut item: T,
            include: &mut bit_vec::Iter,
            out: &mut Vec<Arc<T>>,
        ) {
            let include_item = include.next().unwrap();
            if let Some(children) = item.children_mut() {
                if !include_item {
                    unflatten_items(take(children), include, out);
                    return;
                }
                unflatten_items(take(children), include, children);
            }
            if include_item {
                out.push(Arc::new(item));
            }
        }

        let mut items = Vec::new();
        flatten_children(input.iter().collect(), 0, &mut items);

        let mut include = BitVec::from_elem(items.len(), false);
        let selections = if self.0.is_empty() {
            &[Selection::default()]
        } else {
            self.0.as_slice()
        };
        for selection in selections {
            select_matches(&items, selection, &mut include);
        }

        let mut output = Vec::new();
        unflatten_items(
            input.into_iter().map(Arc::new).collect(),
            &mut include.iter(),
            &mut output,
        );
        output
    }
}

impl FromArgMatches for Criteria {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        let mut this = Self::default();
        this.update_from_arg_matches(matches)?;
        Ok(this)
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
        #[derive(Debug)]
        enum Value {
            Or,
            Classes(EnumSet<Class>),
            Commands(StringMatch),
            Subtypes(StringMatch),
            Labels(StringMatch),
            NthCommands(Vec<usize>),
            Instances(Vec<usize>),
            ShowHidden(bool),
            Errors(bool),
        }

        fn extract<F, T: Clone + Send + Sync + 'static>(
            matches: &ArgMatches,
            id: &str,
            output: &mut BTreeMap<usize, Value>,
            f: F,
        ) where
            F: Fn(T) -> Value,
        {
            if !matches.contains_id(id) || matches.try_get_many::<clap::Id>(id).is_ok() {
                // ignore groups
                return;
            }
            let value_source = matches.value_source(id).expect("id came from matches");
            if value_source != clap::parser::ValueSource::CommandLine {
                // Any other source just gets tacked on at the end (like default values)
                return;
            }
            for (value, index) in matches
                .try_get_many::<T>(id)
                .unwrap()
                .unwrap()
                .zip(matches.indices_of(id).unwrap())
            {
                output.insert(index, f(value.clone()));
            }
        }

        let mut values = BTreeMap::new();
        extract(matches, "_or", &mut values, |_: bool| Value::Or);
        extract(matches, "select", &mut values, Value::Classes);
        extract(matches, "commands", &mut values, Value::Commands);
        extract(matches, "subtypes", &mut values, Value::Subtypes);
        extract(matches, "labels", &mut values, Value::Labels);
        extract(matches, "nth_commands", &mut values, Value::NthCommands);
        extract(matches, "instances", &mut values, Value::Instances);
        extract(matches, "show_hidden", &mut values, Value::ShowHidden);
        extract(matches, "errors", &mut values, Value::Errors);

        if !values.is_empty() {
            let mut selection = Selection::default();
            for value in values.into_values() {
                match value {
                    Value::Or => self.0.push(take(&mut selection)),
                    Value::Classes(classes) => selection.classes = classes,
                    Value::Commands(commands) => selection.commands = commands,
                    Value::Subtypes(subtypes) => selection.subtypes = subtypes,
                    Value::Labels(labels) => selection.labels = labels,
                    Value::NthCommands(nth_commands) => selection.nth_commands = nth_commands,
                    Value::Instances(instances) => selection.instances = instances,
                    Value::ShowHidden(show) => {
                        selection.visible = if show { None } else { Some(true) }
                    }
                    Value::Errors(only) => selection.error = if only { Some(true) } else { None },
                }
            }
            self.0.push(selection);
        }
        Ok(())
    }
}

impl Args for Criteria {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        SelectionArgs::augment_args(cmd.next_help_heading("Input selection options"))
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        Self::augment_args(cmd)
    }
}

/// Show information about SPSS viewer files (SPV files).
#[derive(Args, Clone, Debug)]
struct SelectionArgs {
    /// Classes of objects to include or, with leading `^`, to exclude.  The
    /// supported classes are: charts, headings, logs, models, tables, texts,
    /// trees, warnings, outlineheaders, pagetitle, notes, unknown, other.
    #[arg(long, required = false, value_parser = Selection::parse_classes, action = ArgAction::Append)]
    select: EnumSet<Class>,

    /// Identifiers of commands to include or, with leading `^`, to exclude.
    #[arg(long, required = false, value_parser = StringMatch::from_str, action = ArgAction::Append)]
    commands: StringMatch,

    /// Table subtypes to include or, with leading `^`, to exclude.
    #[arg(long, required = false, value_parser = StringMatch::from_str, action = ArgAction::Append)]
    subtypes: StringMatch,

    /// Labels (table titles) to include or, with leading `^`, to exclude.
    #[arg(long, required = false, value_parser = StringMatch::from_str, action = ArgAction::Append)]
    labels: StringMatch,

    /// Include only objects from the Nth (1-based) command that matches
    /// `--command`.
    #[arg(long, required = false, value_parser = Selection::parse_nth_commands, action = ArgAction::Append)]
    nth_commands: Vec<usize>,

    /// Include only the given instances of an object that matches the other
    /// criteria within a single command.  Each instance may be a number (1 for
    /// the first, and so on), or `last` for the last instance.
    #[arg(long, required = false, value_parser = Selection::parse_instances, action = ArgAction::Append)]
    instances: Vec<usize>,

    /// Include hidden objects in the output (by default, they are excluded)
    #[arg(long, required = false, action = ArgAction::Append, num_args = 0, value_parser = value_parser!(bool), default_missing_value = "true", default_value = "false")]
    show_hidden: bool,

    /// Include only objects that cause an error when read (by default, objects
    /// with and without errors are included).
    #[arg(long, required = false, action = ArgAction::Append, num_args = 0, value_parser = value_parser!(bool), default_missing_value = "true", default_value = "false")]
    errors: bool,

    /// Include only XML and binary member names that match.  Without any member
    /// names, include all objects.
    #[arg(long, required = false, action = ArgAction::Append)]
    pub members: Vec<String>,

    /// Separate two groups of selection options.
    #[arg(long, action = ArgAction::Append, long = "or", num_args = 0, value_parser = value_parser!(bool), default_missing_value = "true", default_value = "false")]
    _or: bool,
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use enumset::EnumSet;

    use crate::output::{
        Class, Criteria, Heading, Item, Selection, StringMatch, pivot::PivotTable,
    };

    #[test]
    fn parse_classes() {
        assert_eq!(Selection::parse_classes("").unwrap(), EnumSet::all());
        assert_eq!(
            Selection::parse_classes("tables").unwrap(),
            EnumSet::only(Class::Tables)
        );
        assert_eq!(
            Selection::parse_classes("tables,pagetitle").unwrap(),
            EnumSet::only(Class::Tables) | EnumSet::only(Class::PageTitle)
        );
        assert_eq!(
            Selection::parse_classes("^tables,pagetitle").unwrap(),
            !(EnumSet::only(Class::Tables) | EnumSet::only(Class::PageTitle))
        );
    }

    #[test]
    fn parse_nth_commands() {
        assert_eq!(Selection::parse_nth_commands("1").unwrap(), vec![1]);
        assert_eq!(
            Selection::parse_nth_commands("1,2,3").unwrap(),
            vec![1, 2, 3]
        );
        assert!(Selection::parse_nth_commands("0").is_err());
    }

    #[test]
    fn parse_instances() {
        assert_eq!(Selection::parse_instances("1").unwrap(), vec![1]);
        assert_eq!(Selection::parse_instances("2,3").unwrap(), vec![2, 3]);
        assert_eq!(Selection::parse_instances("last,1").unwrap(), vec![0, 1]);
        assert!(Selection::parse_instances("0").is_err());
    }

    #[test]
    fn string_matches() {
        assert!(StringMatch::default().matches("xyzzy"));
        assert!(StringMatch::Exclude(Vec::new()).matches("xyzzy"));
        assert!(!StringMatch::Include(Vec::new()).matches("xyzzy"));

        let m = StringMatch::Include(vec![String::from("xyz"), String::from("abc*")]);
        assert!(m.matches("xyz"));
        assert!(!m.matches("xyzzy"));
        assert!(!m.matches("ab"));
        assert!(m.matches("abc"));
        assert!(m.matches("abcd"));

        let m = StringMatch::Exclude(vec![String::from("xyz"), String::from("abc*")]);
        assert!(!m.matches("xyz"));
        assert!(m.matches("xyzzy"));
        assert!(m.matches("ab"));
        assert!(!m.matches("abc"));
        assert!(!m.matches("abcd"));
    }

    #[test]
    fn selections() {
        #[derive(Parser, Debug, PartialEq, Eq)]
        struct Command {
            #[command(flatten)]
            selections: Criteria,
        }

        let args = vec![
            "myprog",
            "--select=tables",
            "--or",
            "--commands=Frequencies,Descriptives",
        ];
        let matches = Command::parse_from(args);
        assert_eq!(
            matches,
            Command {
                selections: Criteria(vec![
                    Selection {
                        classes: EnumSet::only(Class::Tables),
                        ..Selection::default()
                    },
                    Selection {
                        commands: StringMatch::Include(vec![
                            String::from("Frequencies"),
                            String::from("Descriptives")
                        ]),
                        ..Selection::default()
                    }
                ])
            }
        );
    }

    fn regress_item() -> Item {
        [
            Heading::new()
                .into_item()
                .with_label("Set")
                .with_some_command_name("Set"),
            [Heading::new()
                .into_item()
                .with_label("Page Title")
                .with_some_command_name("Title")]
            .into_iter()
            .collect::<Item>()
            .with_label("Title")
            .with_some_command_name("Title"),
            [PivotTable::new([])
                .with_title("Reading 1 record from INLINE.")
                .with_subtype("Fixed Data Records")
                .into_item()
                .with_some_command_name("Data List")]
            .into_iter()
            .collect::<Item>()
            .with_label("Data List")
            .with_some_command_name("Data List"),
            Heading::new()
                .into_item()
                .with_label("Begin Data")
                .with_some_command_name("Begin Data"),
            [PivotTable::new([])
                .with_title("Data List")
                .into_item()
                .with_some_command_name("List")]
            .into_iter()
            .collect::<Item>()
            .with_label("List")
            .with_some_command_name("List"),
        ]
        .into_iter()
        .collect::<Item>()
        .with_label("Output")
    }
}
