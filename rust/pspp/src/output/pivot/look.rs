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

//! Pivot table styles (called "TableLooks" by SPSS).
//!
//! Each [PivotTable] is styled with a [PivotTableStyle], which contains a
//! [Look].  This module contains [Look] and its contents.
//!
//! [PivotTable]: super::PivotTable
//! [PivotTableStyle]: super::PivotTableStyle

// Warn about missing docs, but not for items declared with `#[cfg(test)]`.
#![cfg_attr(not(test), warn(missing_docs))]
#![warn(dead_code)]

use std::{
    fmt::{Debug, Display},
    io::Read,
    ops::{Range, RangeInclusive},
    str::{FromStr, Utf8Error},
    sync::{Arc, OnceLock},
};

use color::{AlphaColor, Rgba8, Srgb, palette::css::TRANSPARENT};
use enum_iterator::Sequence;
use enum_map::{Enum, EnumMap, enum_map};
use quick_xml::{DeError, de::from_str};
use serde::{Deserialize, Serialize, de::Visitor, ser::SerializeStruct};

use crate::{
    output::pivot::{
        Axis2, FootnoteMarkerPosition, FootnoteMarkerType, TableProperties, tlo::parse_tlo,
    },
    util::ToSmallString,
    variable::VarType,
};

/// Areas of a pivot table for styling purposes.
#[derive(Copy, Clone, Debug, Enum, PartialEq, Eq)]
pub enum Area {
    /// Title.
    ///
    /// Displayed above the table.  If a table is split across multiple pages
    /// for printing, the title appears on each page.
    Title,

    /// Caption.
    ///
    /// Displayed below the table.
    Caption,

    /// Footnotes.
    ///
    /// Displayed below the table.
    Footer,

    /// Top-left corner.
    ///
    /// To the left of the column labels, and above the row labels.
    Corner,

    /// Labels.
    Labels(
        /// - [Axis2::X]: Column labels, along the top of the table.
        /// - [Axis2::Y]: Row labels, along the left side of the table.
        Axis2,
    ),

    /// Data cells.
    Data(
        /// This allows styling for even rows and odd rows to differ
        /// arbitrarily, but the SPV file format only distinguishes foreground
        /// and background colors, so any other differences will be lost upon
        /// save.
        RowParity,
    ),

    /// Layer indication.
    Layers,
}

impl Area {
    /// Returns the row parity for [Area::Data], or `None` for other areas.
    pub fn row_parity(self) -> Option<RowParity> {
        match self {
            Area::Data(row_parity) => Some(row_parity),
            _ => None,
        }
    }
}

impl Default for Area {
    fn default() -> Self {
        Self::Data(RowParity::default())
    }
}

impl Display for Area {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Area::Title => write!(f, "title"),
            Area::Caption => write!(f, "caption"),
            Area::Footer => write!(f, "footer"),
            Area::Corner => write!(f, "corner"),
            Area::Labels(axis2) => write!(f, "labels({axis2})"),
            Area::Data(row) => write!(f, "data({row})"),
            Area::Layers => write!(f, "layers"),
        }
    }
}

impl Serialize for Area {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_small_string::<16>())
    }
}

/// Distinguishes [Area::Data] for even-numbered and odd-numbered rows.
#[derive(Copy, Clone, Debug, Default, Enum, PartialEq, Eq)]
pub enum RowParity {
    /// Even-numbered rows.
    ///
    /// The first row is row 0, hence even.
    #[default]
    Even,
    /// Odd-numbered rows.
    Odd,
}

impl From<usize> for RowParity {
    fn from(value: usize) -> Self {
        if value % 2 == 1 {
            Self::Odd
        } else {
            Self::Even
        }
    }
}

impl Display for RowParity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RowParity::Even => write!(f, "even"),
            RowParity::Odd => write!(f, "odd"),
        }
    }
}

/// Table borders for styling purposes.
#[derive(Copy, Clone, Debug, Enum, PartialEq, Eq)]
pub enum Border {
    /// Title.
    Title,

    /// Outer frame.
    OuterFrame(
        /// Which border of the outer frame.
        BoxBorder,
    ),
    /// Inner frame.
    InnerFrame(
        /// Which border of the inner frame.
        BoxBorder,
    ),
    /// Between dimensions.
    Dimension(
        /// Which part between dimensions.
        RowColBorder,
    ),
    /// Between categories.
    Category(
        /// Which part between categories.
        RowColBorder,
    ),
    /// Between the row borders and the data.
    DataLeft,
    /// Between the column borders and the data.
    DataTop,
}

impl Border {
    /// Returns the default [Stroke] for this border.
    pub fn default_stroke(self) -> Stroke {
        match self {
            Self::InnerFrame(_) | Self::DataLeft | Self::DataTop => Stroke::Thick,
            Self::Dimension(
                RowColBorder(HeadingRegion::Columns, _) | RowColBorder(_, Axis2::X),
            )
            | Self::Category(RowColBorder(HeadingRegion::Columns, _)) => Stroke::Solid,
            _ => Stroke::None,
        }
    }
    /// Returns the default [BorderStyle] for this border.
    pub fn default_border_style(self) -> BorderStyle {
        BorderStyle {
            stroke: self.default_stroke(),
            color: Color::BLACK,
        }
    }

    /// Returns an alternative border for this one.
    pub fn fallback(self) -> Option<Self> {
        if let Self::Dimension(row_col_border) = self {
            Some(Self::Category(row_col_border))
        } else {
            None
        }
    }

    /// Returns all the default borders.
    pub fn default_borders() -> EnumMap<Border, BorderStyle> {
        EnumMap::from_fn(Border::default_border_style)
    }
}

impl Display for Border {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Border::Title => write!(f, "title"),
            Border::OuterFrame(box_border) => write!(f, "outer_frame({box_border})"),
            Border::InnerFrame(box_border) => write!(f, "inner_frame({box_border})"),
            Border::Dimension(row_col_border) => write!(f, "dimension({row_col_border})"),
            Border::Category(row_col_border) => write!(f, "category({row_col_border})"),
            Border::DataLeft => write!(f, "data(left)"),
            Border::DataTop => write!(f, "data(top)"),
        }
    }
}

impl Serialize for Border {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_small_string::<32>())
    }
}

/// The borders on a box.
#[derive(Copy, Clone, Debug, Enum, PartialEq, Eq, Serialize, Sequence)]
#[serde(rename_all = "snake_case")]
pub enum BoxBorder {
    /// Left side.
    Left,
    /// Top.
    Top,
    /// Right side.
    Right,
    /// Bottom.
    Bottom,
}

impl BoxBorder {
    fn as_str(&self) -> &'static str {
        match self {
            BoxBorder::Left => "left",
            BoxBorder::Top => "top",
            BoxBorder::Right => "right",
            BoxBorder::Bottom => "bottom",
        }
    }
}

impl Display for BoxBorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Borders between rows and columns.
#[derive(Copy, Clone, Debug, Enum, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RowColBorder(
    /// Row or column headings.
    pub HeadingRegion,
    /// Horizontal ([Axis2::X]) or vertical ([Axis2::Y]) borders.
    pub Axis2,
);

impl Display for RowColBorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.0, self.1)
    }
}

/// Sizing for rows or columns of a rendered table.
///
/// The comments below talk about columns and their widths but they apply
/// equally to rows and their heights.
#[derive(Default, Clone, Debug, Serialize)]
pub struct Sizing {
    /// Specific column widths, in 1/96" units.
    pub widths: Vec<i32>,

    /// Specific page breaks: 0-based columns after which a page break must
    /// occur, e.g. a value of 1 requests a break after the second column.
    pub breaks: Vec<usize>,

    /// Keeps: columns to keep together on a page if possible.
    pub keeps: Vec<Range<usize>>,
}

/// Core styling for a pivot table.
///
/// The division between `Look` and [PivotTableStyle] is fairly arbitrary.  The
/// ultimate reason for the division is simply because that's how SPSS
/// documentation and file formats do it.
///
/// A `Look` can be read from standalone files in [XML] and [binary] formats and
/// extracted from [PivotTable]s, which in turn can be read from [SPV files].
///
/// [XML]: Self::from_xml
/// [binary]: Self::from_binary
/// [PivotTable]: super::PivotTable
/// [PivotTableStyle]: super::PivotTableStyle
/// [SPV files]: crate::spv
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Look {
    /// Optional name for this `Look`.
    ///
    /// This allows building a catalog of `Look`s based on more than their
    /// filenames.
    pub name: Option<String>,

    /// Whether to hide rows or columns whose cells are all empty.
    pub hide_empty: bool,

    /// Where to place [Group] labels.
    ///
    /// [Group]: super::Group
    pub row_label_position: LabelPosition,

    /// Ranges of column widths in the two heading regions, in 1/96" units.
    pub heading_widths: EnumMap<HeadingRegion, RangeInclusive<isize>>,

    /// Kind of markers to use for footnotes.
    pub footnote_marker_type: FootnoteMarkerType,

    /// Where to put the footnote markers.
    pub footnote_marker_position: FootnoteMarkerPosition,

    /// Styles for areas of the pivot table.
    pub areas: EnumMap<Area, AreaStyle>,

    /// Styles for borders in the pivot table.
    pub borders: EnumMap<Border, BorderStyle>,

    /// Whether to print all layers.
    ///
    /// If true, all table layers are printed sequentially,
    /// If false, only the current layer is printed.
    ///
    /// This affects only printing.  On-screen display shows just one layer at a
    /// time.
    pub print_all_layers: bool,

    /// If true, print each layer on its own page.
    pub paginate_layers: bool,

    /// If `shrink_to_fit[Axis2::X]`, then tables wider than the page are scaled
    /// to fit horizontally.
    ///
    /// If `shrink_to_fit[Axis2::Y]`, then tables longer than the page are
    /// scaled to fit vertically.
    pub shrink_to_fit: EnumMap<Axis2, bool>,

    /// When to show `continuation`:
    ///
    /// - `show_continuations[0]`: Whether to show `continuation` at the top of
    ///   a table that is continued from the previous page.
    ///
    /// - `show_continuations[1]`: Whether to show `continuation` at the bottom
    ///   of a table that continues onto the next page.
    pub show_continuations: [bool; 2],

    /// Text that can be shown at the top or bottom of a table that continues
    /// across multiple pages.
    pub continuation: Option<String>,

    /// Minimum number of rows or columns to put in one part of a table that is
    /// broken across pages.
    pub n_orphan_lines: usize,
}

impl Look {
    /// Returns this look with `omit_empty` set as provided.
    pub fn with_omit_empty(mut self, omit_empty: bool) -> Self {
        self.hide_empty = omit_empty;
        self
    }
    /// Returns this look with `row_label_position` set as provided.
    pub fn with_row_label_position(mut self, row_label_position: LabelPosition) -> Self {
        self.row_label_position = row_label_position;
        self
    }
    /// Returns this look with `borders` set as provided.
    pub fn with_borders(mut self, borders: EnumMap<Border, BorderStyle>) -> Self {
        self.borders = borders;
        self
    }
}

impl Default for Look {
    fn default() -> Self {
        Self {
            name: None,
            hide_empty: true,
            row_label_position: LabelPosition::default(),
            heading_widths: EnumMap::from_fn(|region| match region {
                HeadingRegion::Rows => 48..=96,
                HeadingRegion::Columns => 48..=160,
            }),
            footnote_marker_type: FootnoteMarkerType::default(),
            footnote_marker_position: FootnoteMarkerPosition::default(),
            areas: EnumMap::from_fn(AreaStyle::default_for_area),
            borders: Border::default_borders(),
            print_all_layers: false,
            paginate_layers: false,
            shrink_to_fit: EnumMap::from_fn(|_| false),
            show_continuations: [false, false],
            continuation: None,
            n_orphan_lines: 0,
        }
    }
}

/// Error type returned by [Look] methods that parse a file.
#[derive(thiserror::Error, Debug)]
pub enum ParseLookError {
    /// [quick_xml] deserialization errors.
    #[error(transparent)]
    XmlError(
        /// Inner error.
        #[from]
        DeError,
    ),

    /// UTF-8 decoding error.
    #[error(transparent)]
    Utf8Error(
        /// Inner error.
        #[from]
        Utf8Error,
    ),

    /// [binrw] deserialization error.
    #[error(transparent)]
    BinError(
        /// Inner error.
        #[from]
        binrw::Error,
    ),

    /// I/O error.
    #[error(transparent)]
    IoError(
        /// Inner error.
        #[from]
        std::io::Error,
    ),
}

impl Look {
    /// Returns a globally shared copy of [Look::default].
    pub fn shared_default() -> Arc<Look> {
        static LOOK: OnceLock<Arc<Look>> = OnceLock::new();
        LOOK.get_or_init(|| Arc::new(Look::default())).clone()
    }

    /// Parses `xml` as an XML-formatted `Look`, as found in [`.stt` files].
    ///
    /// [`.stt` files]: https://pspp.benpfaff.org/manual/tablelook.html#the-stt-format
    pub fn from_xml(xml: &str) -> Result<Self, ParseLookError> {
        Ok(from_str::<TableProperties>(xml)
            .map_err(ParseLookError::from)?
            .into())
    }

    /// Parses `xml` as a binary-formatted `Look`, as found in [`.tlo` files].
    ///
    /// # Obsolescence
    ///
    /// The `.tlo` format is obsolete.  PSPP only supports it as an input
    /// format.
    ///
    /// [`.tlo` files]: https://pspp.benpfaff.org/manual/tablelook.html#the-tlo-format
    pub fn from_binary(tlo: &[u8]) -> Result<Self, ParseLookError> {
        parse_tlo(tlo).map_err(ParseLookError::from)
    }

    /// Parses `data` as the binary or XML format of a `Look`, automatically
    /// detecting which format.
    pub fn from_data(data: &[u8]) -> Result<Self, ParseLookError> {
        if data.starts_with(b"\xff\xff\0\0") {
            Self::from_binary(data)
        } else {
            Self::from_xml(str::from_utf8(data).map_err(ParseLookError::from)?)
        }
    }

    /// Reads a [Look] in binary or XML format from `reader`.
    pub fn from_reader<R>(mut reader: R) -> Result<Self, ParseLookError>
    where
        R: Read,
    {
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer)?;
        Self::from_data(&buffer)
    }
}

/// Position for [Group] labels.
///
/// [Group]: super::Group
#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum LabelPosition {
    /// Hierarchically enclosing the categories.
    ///
    /// For column labels, group labels appear above the categories.  For row
    /// labels, group labels appear to the left of the categories.
    ///
    /// ```text
    /// ┌────┬──────────────┐   ┌─────────┬──────────┐
    /// │    │    nested    │   │         │ columns  │
    /// │    ├────┬────┬────┤   ├──────┬──┼──────────┤
    /// │    │ a1 │ a2 │ a3 │   │      │a1│...data...│
    /// ├────┼────┼────┼────┤   │nested│a2│...data...│
    /// │    │data│data│data│   │      │a3│...data...│
    /// │    │ .  │ .  │ .  │   └──────┴──┴──────────┘
    /// │rows│ .  │ .  │ .  │
    /// │    │ .  │ .  │ .  │
    /// └────┴────┴────┴────┘
    /// ```
    #[serde(rename = "nested")]
    Nested,

    /// In the corner (row labels only).
    ///
    /// ```text
    /// ┌──────┬──────────┐
    /// │corner│ columns  │
    /// ├──────┼──────────┤
    /// │    a1│...data...│
    /// │    a2│...data...│
    /// │    a3│...data...│
    /// └──────┴──────────┘
    /// ```
    #[default]
    #[serde(rename = "inCorner")]
    Corner,
}

/// The heading region of a rendered pivot table:
///
/// ```text
/// ┌──────────────────┬─────────────────────────────────────────────────┐
/// │                  │                  column headings                │
/// │                  ├─────────────────────────────────────────────────┤
/// │      corner      │                                                 │
/// │       and        │                                                 │
/// │   row headings   │                      data                       │
/// │                  │                                                 │
/// │                  │                                                 │
/// └──────────────────┴─────────────────────────────────────────────────┘
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Enum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadingRegion {
    /// Headings for labeling rows.
    ///
    /// These appear on the left side of the pivot table, including the top-left
    /// corner area.
    Rows,

    /// Headings for labeling columns.
    ///
    /// These appear along the top of the pivot table, excluding the top-left
    /// corner area.
    Columns,
}

impl HeadingRegion {
    /// Returns "rows" or "columns".
    pub fn as_str(&self) -> &'static str {
        match self {
            HeadingRegion::Rows => "rows",
            HeadingRegion::Columns => "columns",
        }
    }
}

impl Display for HeadingRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<Axis2> for HeadingRegion {
    fn from(axis: Axis2) -> Self {
        match axis {
            Axis2::X => HeadingRegion::Columns,
            Axis2::Y => HeadingRegion::Rows,
        }
    }
}

/// Default style for cells in an [Area].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AreaStyle {
    /// Cell style for the area.
    pub cell_style: CellStyle,
    /// Font style for the area.
    pub font_style: FontStyle,
}

impl AreaStyle {
    /// Returns the default style for `area`.
    pub fn default_for_area(area: Area) -> Self {
        Self {
            cell_style: CellStyle::default_for_area(area),
            font_style: FontStyle::default_for_area(area),
        }
    }
}

/// Style for the cells that contain a [Value].
///
/// The division between [CellStyle] and [FontStyle] isn't particularly
/// meaningful but it matches SPSS file formats.
///
/// [Value]: super::value::Value
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CellStyle {
    /// Horizontal alignment.
    ///
    /// `None` means "mixed" alignment: align strings to the left, numbers to
    /// the right.
    pub horz_align: Option<HorzAlign>,

    /// Vertical alignment.
    pub vert_align: VertAlign,

    /// Margins in 1/96" units:
    ///
    /// - `margins[Axis2::X][0]` is the left margin.
    /// - `margins[Axis2::X][1]` is the right margin.
    /// - `margins[Axis2::Y][0]` is the top margin.
    /// - `margins[Axis2::Y][1]` is the bottom margin.
    pub margins: EnumMap<Axis2, [i32; 2]>,
}

impl Default for CellStyle {
    fn default() -> Self {
        Self::default_for_area(Area::default())
    }
}

impl CellStyle {
    /// Returns the default cell style for `area`.
    pub fn default_for_area(area: Area) -> Self {
        use HorzAlign::*;
        use VertAlign::*;
        let (horz_align, vert_align, hmargins, vmargins) = match area {
            Area::Title => (Some(Center), Middle, [8, 11], [1, 8]),
            Area::Caption => (Some(Left), Top, [8, 11], [1, 1]),
            Area::Footer => (Some(Left), Top, [11, 8], [2, 3]),
            Area::Corner => (Some(Left), Bottom, [8, 11], [1, 1]),
            Area::Labels(Axis2::X) => (Some(Center), Bottom, [8, 11], [1, 3]),
            Area::Labels(Axis2::Y) => (Some(Left), Top, [8, 11], [1, 3]),
            Area::Data(_) => (None, Top, [8, 11], [1, 1]),
            Area::Layers => (Some(Left), Bottom, [8, 11], [1, 3]),
        };
        Self {
            horz_align,
            vert_align,
            margins: enum_map! { Axis2::X => hmargins, Axis2::Y => vmargins },
        }
    }
    /// Returns the cell style with `horz_align` set as specified.
    pub fn with_horz_align(self, horz_align: Option<HorzAlign>) -> Self {
        Self { horz_align, ..self }
    }
    /// Returns the cell style with `vert_align` set as specified.
    pub fn with_vert_align(self, vert_align: VertAlign) -> Self {
        Self { vert_align, ..self }
    }
    /// Returns the cell style with `margins` set as specified.
    pub fn with_margins(self, margins: EnumMap<Axis2, [i32; 2]>) -> Self {
        Self { margins, ..self }
    }
}

/// Horizontal alignment of text.
///
/// "Mixed" alignment is implemented at a higher level using
/// `Option<HorzAlign>`.
#[derive(Copy, Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HorzAlign {
    /// Right aligned.
    Right,

    /// Left aligned.
    Left,

    /// Centered.
    Center,

    /// Align the decimal point at the specified position.
    Decimal {
        /// Decimal offset from the right side of the cell, in 1/96" units.
        offset: f64,
    },
}

impl HorzAlign {
    /// Returns the [HorzAlign] to use for "mixed alignment" based on the
    /// variable type.
    pub fn for_mixed(var_type: VarType) -> Self {
        match var_type {
            VarType::Numeric => Self::Right,
            VarType::String => Self::Left,
        }
    }

    /// Returns this alignment as a static string.
    ///
    /// Decimal alignment doesn't have a static string representation.
    pub fn as_str(&self) -> Option<&'static str> {
        match self {
            HorzAlign::Right => Some("right"),
            HorzAlign::Left => Some("left"),
            HorzAlign::Center => Some("center"),
            HorzAlign::Decimal { .. } => None,
        }
    }
}

/// Unknown horizontal alignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UnknownHorzAlign;

impl FromStr for HorzAlign {
    type Err = UnknownHorzAlign;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("left") {
            Ok(Self::Left)
        } else if s.eq_ignore_ascii_case("center") {
            Ok(Self::Center)
        } else if s.eq_ignore_ascii_case("right") {
            Ok(Self::Right)
        } else {
            Err(UnknownHorzAlign)
        }
    }
}

/// Vertical alignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VertAlign {
    /// Top alignment.
    Top,

    /// Centered,
    Middle,

    /// Bottom alignment.
    Bottom,
}

/// Style of the font used in a [Value].
///
/// The division between [CellStyle] and [FontStyle] isn't particularly
/// meaningful but it matches SPSS file formats.
///
/// [Value]: super::value::Value
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontStyle {
    /// **Bold**
    pub bold: bool,

    /// *Italic*
    pub italic: bool,

    /// <u>Underline</u>
    pub underline: bool,

    /// Typeface.
    pub font: String,

    /// Foreground color.
    pub fg: Color,

    /// Background color.
    pub bg: Color,

    /// In 1/72" units.
    pub size: i32,
}

impl Default for FontStyle {
    fn default() -> Self {
        FontStyle {
            bold: false,
            italic: false,
            underline: false,
            font: String::from("Sans Serif"),
            fg: Color::BLACK,
            bg: Color::WHITE,
            size: 9,
        }
    }
}

impl FontStyle {
    /// Returns the default font style for `area`.
    pub fn default_for_area(area: Area) -> Self {
        Self::default().with_bold(area == Area::Title)
    }
    /// Returns the font with its `size` set as specified.
    pub fn with_size(self, size: i32) -> Self {
        Self { size, ..self }
    }
    /// Returns the font with `bold` set as specified.
    pub fn with_bold(self, bold: bool) -> Self {
        Self { bold, ..self }
    }
    /// Returns the font with `italic` set as specified.
    pub fn with_italic(self, italic: bool) -> Self {
        Self { italic, ..self }
    }
    /// Returns the font with `underline` set as specified.
    pub fn with_underline(self, underline: bool) -> Self {
        Self { underline, ..self }
    }
    /// Returns the font with `font` set as specified.
    pub fn with_font(self, font: impl Into<String>) -> Self {
        Self {
            font: font.into(),
            ..self
        }
    }
    /// Returns the font with `fg` set as specified.
    pub fn with_fg(self, fg: Color) -> Self {
        Self { fg, ..self }
    }
    /// Returns the font with `bg` set as specified.
    pub fn with_bg(self, fg: Color) -> Self {
        Self { fg, ..self }
    }
}

/// Color used in [FontStyle].
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Color {
    /// Alpha channel.
    ///
    /// 255 is opaque, 0 is transparent.
    pub alpha: u8,

    /// Red.
    pub r: u8,

    /// Green.
    pub g: u8,

    /// Blue.
    pub b: u8,
}

impl Color {
    /// Black.
    pub const BLACK: Color = Color::new(0, 0, 0);
    /// White.
    pub const WHITE: Color = Color::new(255, 255, 255);
    /// Red.
    pub const RED: Color = Color::new(255, 0, 0);
    /// Green.
    pub const GREEN: Color = Color::new(0, 255, 0);
    /// Blue.
    pub const BLUE: Color = Color::new(0, 0, 255);
    /// Transparent.
    pub const TRANSPARENT: Color = Color::new(0, 0, 0).with_alpha(0);

    /// Returns an opaque color with the given red, green, and blue values.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self {
            alpha: 255,
            r,
            g,
            b,
        }
    }

    /// Returns an opaque color taken from `rgb`.
    pub const fn new_u32(rgb: u32) -> Self {
        Self::new((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    /// Returns this color with the alpha channel set as specified.
    pub const fn with_alpha(self, alpha: u8) -> Self {
        Self { alpha, ..self }
    }

    /// Returns an opaque version of this color.
    pub const fn without_alpha(self) -> Self {
        self.with_alpha(255)
    }

    /// Displays opaque colors as `#rrggbb` and others as `rgb(r, g, b, alpha)`.
    pub fn display_css(&self) -> impl Display {
        ColorDisplayCss(*self)
    }

    /// Returns the red, green, and blue channels of this color.
    pub fn into_rgb(&self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }

    /// Returns 16-bit versions of the red, green, and blue channels of this
    /// color.
    pub fn into_rgb16(&self) -> (u16, u16, u16) {
        (
            self.r as u16 * 257,
            self.g as u16 * 257,
            self.b as u16 * 257,
        )
    }
}

impl Debug for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_css())
    }
}

impl From<Rgba8> for Color {
    fn from(Rgba8 { r, g, b, a }: Rgba8) -> Self {
        Self::new(r, g, b).with_alpha(a)
    }
}

impl FromStr for Color {
    type Err = color::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        fn is_bare_hex(s: &str) -> bool {
            let s = s.trim();
            s.chars().count() == 6 && s.chars().all(|c| c.is_ascii_hexdigit())
        }
        let color: AlphaColor<Srgb> = match s.parse() {
            Err(_) if is_bare_hex(s) => ("#".to_owned() + s).parse(),
            Err(_) if s.trim().eq_ignore_ascii_case("transparent") => Ok(TRANSPARENT),
            other => other,
        }?;
        Ok(color.to_rgba8().into())
    }
}

impl Serialize for Color {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.display_css().to_small_string::<32>())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ColorVisitor;

        impl<'de> Visitor<'de> for ColorVisitor {
            type Value = Color;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("\"#rrggbb\" or \"rrggbb\" or web color name")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                v.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_str(ColorVisitor)
    }
}

/// A structure for formatting a [Color] in a CSS-compatible format.
///
/// See [Color::display_css].
struct ColorDisplayCss(Color);

impl Display for ColorDisplayCss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Color { alpha, r, g, b } = self.0;
        match alpha {
            255 => write!(f, "#{r:02x}{g:02x}{b:02x}"),
            _ => write!(f, "rgb({r}, {g}, {b}, {:.2})", alpha as f64 / 255.0),
        }
    }
}

/// Style for drawing a border in a pivot table.
#[derive(Copy, Clone, Debug, PartialEq, Deserialize)]
pub struct BorderStyle {
    /// The kind of line to draw.
    #[serde(rename = "@borderStyleType")]
    pub stroke: Stroke,

    /// Line color.
    #[serde(rename = "@color")]
    pub color: Color,
}

impl Serialize for BorderStyle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = serializer.serialize_struct("BorderStyle", 2)?;
        s.serialize_field("stroke", &self.stroke)?;
        s.serialize_field("color", &self.color)?;
        s.end()
    }
}

impl From<Stroke> for BorderStyle {
    fn from(value: Stroke) -> Self {
        Self::new(value)
    }
}

impl BorderStyle {
    /// Returns a black border style with the given `stroke`.
    pub const fn new(stroke: Stroke) -> Self {
        Self {
            stroke,
            color: Color::BLACK,
        }
    }

    /// Returns a border style with no line.
    pub const fn none() -> Self {
        Self::new(Stroke::None)
    }

    /// Returns a solid, black border style.
    pub const fn solid() -> Self {
        Self::new(Stroke::Solid)
    }

    /// Returns whether this border style has no line.
    pub fn is_none(&self) -> bool {
        self.stroke.is_none()
    }

    /// Returns this border style with the given `stroke`.
    pub fn with_stroke(self, stroke: Stroke) -> Self {
        Self { stroke, ..self }
    }

    /// Returns this border style with the given `color`.
    pub fn with_color(self, color: Color) -> Self {
        Self { color, ..self }
    }

    /// Returns a border style that "combines" the two arguments, that is, that
    /// gives a reasonable choice for a rule for different reasons should have
    /// both styles.
    pub fn combine(self, other: BorderStyle) -> Self {
        Self {
            stroke: self.stroke.combine(other.stroke),
            color: self.color,
        }
    }
}

/// A line style for borders in a pivot table.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Enum, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stroke {
    /// No line.
    None,
    /// Solid line.
    Solid,
    /// Dashed line.
    Dashed,
    /// Thick solid line.
    Thick,
    /// Thin solid line.
    Thin,
    /// Two lines.
    Double,
}

impl Stroke {
    /// Return whether this stroke is [Stroke::None].
    pub fn is_none(&self) -> bool {
        self == &Self::None
    }

    /// Returns a stroke that "combines" the two arguments, that is, that gives
    /// a reasonable stroke choice for a rule for different reasons should have
    /// both styles.
    pub fn combine(self, other: Stroke) -> Self {
        self.max(other)
    }
}
