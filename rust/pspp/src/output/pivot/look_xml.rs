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

use std::{fmt::Debug, num::ParseFloatError, str::FromStr};

use enum_map::{EnumMap, enum_map};
use serde::{Deserialize, de::Visitor};

use crate::output::pivot::{
    Axis2, FootnoteMarkerPosition, FootnoteMarkerType,
    look::{
        self, Area, AreaStyle, Border, BoxBorder, Color, HeadingRegion, HorzAlign, LabelPosition,
        Look, RowColBorder, RowParity, Stroke, VertAlign,
    },
};
use thiserror::Error as ThisError;

/// An XML TableLook.
#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TableProperties {
    #[serde(rename = "@name")]
    name: Option<String>,
    general_properties: GeneralProperties,
    footnote_properties: FootnoteProperties,
    cell_format_properties: CellFormatProperties,
    border_properties: BorderProperties,
    printing_properties: PrintingProperties,
}

impl From<TableProperties> for Look {
    fn from(table_properties: TableProperties) -> Self {
        Self {
            name: table_properties.name,
            hide_empty: table_properties.general_properties.hide_empty_rows,
            row_label_position: table_properties.general_properties.row_label_position,
            heading_widths: enum_map! {
                HeadingRegion::Columns => table_properties.general_properties.minimum_column_width..=table_properties.general_properties.maximum_column_width,
                HeadingRegion::Rows => table_properties.general_properties.minimum_row_width..=table_properties.general_properties.maximum_row_width,
            }.map(|_k, r|r.start().as_px_isize()..=r.end().as_px_isize()),
            footnote_marker_type: table_properties.footnote_properties.marker_type,
            footnote_marker_position: table_properties.footnote_properties.marker_position,
            areas: EnumMap::from_fn(|area| {
                table_properties.cell_format_properties.get_style(area).as_area_style(area)
            }),
            borders: table_properties.border_properties.decode(),
            print_all_layers: table_properties.printing_properties.print_all_layers,
            paginate_layers: table_properties
                .printing_properties
                .print_each_layer_on_separate_page,
            shrink_to_fit: enum_map! {
                Axis2::X => table_properties.printing_properties.rescale_wide_table_to_fit_page,
                Axis2::Y => table_properties.printing_properties.rescale_long_table_to_fit_page,
            },
            show_continuations: [ table_properties
                                  .printing_properties
                                  .continuation_text_at_top,
                                  table_properties
                                  .printing_properties
                                  .continuation_text_at_bottom],
            continuation: {
                let text = table_properties.printing_properties.continuation_text;
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            },
            n_orphan_lines: table_properties
                .printing_properties
                .window_orphan_lines
                .try_into()
                .unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct GeneralProperties {
    #[serde(rename = "@hideEmptyRows")]
    hide_empty_rows: bool,

    #[serde(rename = "@maximumColumnWidth")]
    maximum_column_width: Length,

    #[serde(rename = "@minimumColumnWidth")]
    minimum_column_width: Length,

    #[serde(rename = "@maximumRowWidth")]
    maximum_row_width: Length,

    #[serde(rename = "@minimumRowWidth")]
    minimum_row_width: Length,

    #[serde(rename = "@rowDimensionLabels")]
    row_label_position: LabelPosition,
}

impl Default for GeneralProperties {
    fn default() -> Self {
        Self {
            hide_empty_rows: true,
            maximum_column_width: Length(1.0),
            minimum_column_width: Length(0.5),
            maximum_row_width: Length(5.0 / 3.0),
            minimum_row_width: Length(0.5),
            row_label_position: LabelPosition::Corner,
        }
    }
}

#[derive(Clone, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
struct FootnoteProperties {
    #[serde(rename = "@markerPosition")]
    marker_position: FootnoteMarkerPosition,

    #[serde(rename = "@numberFormat")]
    marker_type: FootnoteMarkerType,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CellFormatProperties {
    caption: CellStyleHolder,
    column_labels: CellStyleHolder,
    corner_labels: CellStyleHolder,
    data: CellStyleHolder,
    footnotes: CellStyleHolder,
    layers: CellStyleHolder,
    row_labels: CellStyleHolder,
    title: CellStyleHolder,
}

impl CellFormatProperties {
    fn get_style(&self, area: Area) -> &CellStyle {
        match area {
            Area::Title => &self.title.style,
            Area::Caption => &self.caption.style,
            Area::Footer => &self.footnotes.style,
            Area::Corner => &self.corner_labels.style,
            Area::Labels(Axis2::X) => &self.column_labels.style,
            Area::Labels(Axis2::Y) => &self.row_labels.style,
            Area::Data(_) => &self.data.style,
            Area::Layers => &self.layers.style,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CellStyleHolder {
    style: CellStyle,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct CellStyle {
    #[serde(rename = "@alternatingColor")]
    alternating_color: Option<Color>,
    #[serde(rename = "@alternatingTextColor")]
    alternating_text_color: Option<Color>,
    #[serde(rename = "@color")]
    color: Option<Color>,
    #[serde(rename = "@color2")]
    color2: Option<Color>,
    #[serde(rename = "@font-family")]
    font_family: Option<String>,
    #[serde(rename = "@font-size")]
    font_size: Option<Length>,
    #[serde(rename = "@font-style")]
    font_style: Option<FontStyle>,
    #[serde(rename = "@font-weight")]
    font_weight: Option<FontWeight>,
    #[serde(rename = "@font-underline")]
    font_underline: Option<FontUnderline>,
    #[serde(rename = "@labelLocationVertical")]
    label_location_vertical: Option<LabelLocationVertical>,
    #[serde(rename = "@margin-bottom")]
    margin_bottom: Option<Length>,
    #[serde(rename = "@margin-left")]
    margin_left: Option<Length>,
    #[serde(rename = "@margin-right")]
    margin_right: Option<Length>,
    #[serde(rename = "@margin-top")]
    margin_top: Option<Length>,
    #[serde(rename = "@textAlignment")]
    text_alignment: Option<TextAlignment>,
    #[serde(rename = "@decimal-offset")]
    decimal_offset: Option<Length>,
}

impl CellStyle {
    fn default_area_style(area: Area) -> AreaStyle {
        use HorzAlign::*;
        use VertAlign::*;
        const BLACKISH: Color = Color::new(0x01, 0x02, 0x05);
        const WHITE: Color = Color::WHITE;
        const DARK_BLUE: Color = Color::new(0x26, 0x4a, 0x60);
        const LIGHT_GRAY: Color = Color::new(0xe0, 0xe0, 0xe0);
        const VERY_LIGHT_GRAY: Color = Color::new(0xf9, 0xf9, 0xfb);
        let (horz_align, vert_align, hmargins, vmargins, fg, bg, size) = match area {
            Area::Title => (Some(Center), Middle, [6, 8], [1, 6], BLACKISH, WHITE, 11),
            Area::Caption => (Some(Left), Top, [6, 8], [1, 1], BLACKISH, WHITE, 9),
            Area::Footer => (Some(Left), Top, [18, 18], [2, 3], BLACKISH, WHITE, 9),
            Area::Corner => (Some(Left), Bottom, [6, 8], [3, 1], DARK_BLUE, WHITE, 9),
            Area::Labels(Axis2::X) => (Some(Center), Bottom, [6, 8], [2, 2], DARK_BLUE, WHITE, 9),
            Area::Labels(Axis2::Y) => (Some(Left), Top, [6, 8], [3, 2], DARK_BLUE, LIGHT_GRAY, 9),
            Area::Data(_) => (None, Top, [6, 8], [3, 2], DARK_BLUE, VERY_LIGHT_GRAY, 9),
            Area::Layers => (Some(Left), Bottom, [6, 8], [1, 3], BLACKISH, WHITE, 9),
        };
        let bold = area == Area::Title;
        AreaStyle {
            cell_style: look::CellStyle {
                horz_align,
                vert_align,
                margins: enum_map! {
                    Axis2::X => hmargins,
                    Axis2::Y => vmargins,
                },
            },
            font_style: look::FontStyle {
                bold,
                italic: false,
                underline: false,
                font: String::from("SansSerif"),
                fg,
                bg,
                size,
            },
        }
    }

    fn as_area_style(&self, area: Area) -> AreaStyle {
        let mut style = Self::default_area_style(area);

        if let Some(text_alignment) = self.text_alignment {
            style.cell_style.horz_align = match text_alignment {
                TextAlignment::Left => Some(HorzAlign::Left),
                TextAlignment::Right => Some(HorzAlign::Right),
                TextAlignment::Center => Some(HorzAlign::Center),
                TextAlignment::Decimal => Some(HorzAlign::Decimal {
                    offset: self.decimal_offset.map_or(0.0, |offset| offset.as_px_f64()),
                }),
                TextAlignment::Mixed => None,
            };
        }

        if let Some(label_location_vertical) = self.label_location_vertical {
            style.cell_style.vert_align = match label_location_vertical {
                LabelLocationVertical::Positive => VertAlign::Top,
                LabelLocationVertical::Negative => VertAlign::Bottom,
                LabelLocationVertical::Center => VertAlign::Middle,
            };
        }

        if let Some(margin_left) = self.margin_left {
            style.cell_style.margins[Axis2::X][0] = margin_left.as_px_i32();
        }
        if let Some(margin_right) = self.margin_right {
            style.cell_style.margins[Axis2::X][1] = margin_right.as_px_i32();
        }
        if let Some(margin_top) = self.margin_top {
            style.cell_style.margins[Axis2::Y][0] = margin_top.as_px_i32();
        }
        if let Some(margin_bottom) = self.margin_bottom {
            style.cell_style.margins[Axis2::Y][1] = margin_bottom.as_px_i32();
        }

        if let Some(font_weight) = self.font_weight {
            style.font_style.bold = font_weight == FontWeight::Bold;
        }
        if let Some(font_style) = self.font_style {
            style.font_style.italic = font_style == FontStyle::Italic;
        }
        if let Some(font_underline) = self.font_underline {
            style.font_style.underline = font_underline == FontUnderline::Underline;
        }
        if let Some(font_family) = &self.font_family {
            style.font_style.font = font_family.clone();
        }

        let (fg, bg) = if area.row_parity() == Some(RowParity::Odd)
            && (self.alternating_text_color.is_some() || self.alternating_color.is_some())
        {
            (self.alternating_text_color, self.alternating_color)
        } else {
            (self.color, self.color2)
        };
        if fg.is_some() || bg.is_some() {
            style.font_style.fg = fg.unwrap_or(Color::BLACK);
            style.font_style.bg = bg.unwrap_or(Color::WHITE);
        };
        if let Some(font_size) = self.font_size {
            style.font_style.size = font_size.as_pt_i32();
        }
        style
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FontStyle {
    #[default]
    Regular,
    Italic,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FontWeight {
    #[default]
    Regular,
    Bold,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FontUnderline {
    #[default]
    None,
    Underline,
}

#[derive(Copy, Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
enum TextAlignment {
    Left,
    Right,
    Center,
    Decimal,
    #[default]
    Mixed,
}

#[derive(Copy, Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
enum LabelLocationVertical {
    /// Top.
    #[default]
    Positive,

    /// Bottom.
    Negative,

    /// Center.
    Center,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct BorderProperties {
    bottom_inner_frame: Option<BorderStyle>,
    bottom_outer_frame: Option<BorderStyle>,
    data_area_left: Option<BorderStyle>,
    data_area_top: Option<BorderStyle>,
    horizontal_category_border_columns: Option<BorderStyle>,
    horizontal_category_border_rows: Option<BorderStyle>,
    horizontal_dimension_border_columns: Option<BorderStyle>,
    horizontal_dimension_border_rows: Option<BorderStyle>,
    left_inner_frame: Option<BorderStyle>,
    left_outer_frame: Option<BorderStyle>,
    right_inner_frame: Option<BorderStyle>,
    right_outer_frame: Option<BorderStyle>,
    title_layer_separator: Option<BorderStyle>,
    top_inner_frame: Option<BorderStyle>,
    top_outer_frame: Option<BorderStyle>,
    vertical_category_border_columns: Option<BorderStyle>,
    vertical_category_border_rows: Option<BorderStyle>,
    vertical_dimension_border_rows: Option<BorderStyle>,
    vertical_dimension_border_columns: Option<BorderStyle>,
}

impl BorderProperties {
    fn get_style(&self, border: Border) -> &Option<BorderStyle> {
        match border {
            Border::Title => &self.title_layer_separator,
            Border::OuterFrame(BoxBorder::Left) => &self.left_outer_frame,
            Border::OuterFrame(BoxBorder::Top) => &self.top_outer_frame,
            Border::OuterFrame(BoxBorder::Right) => &self.right_outer_frame,
            Border::OuterFrame(BoxBorder::Bottom) => &self.bottom_outer_frame,
            Border::InnerFrame(BoxBorder::Left) => &self.left_inner_frame,
            Border::InnerFrame(BoxBorder::Top) => &self.top_inner_frame,
            Border::InnerFrame(BoxBorder::Right) => &self.right_inner_frame,
            Border::InnerFrame(BoxBorder::Bottom) => &self.bottom_inner_frame,
            Border::Dimension(RowColBorder(HeadingRegion::Columns, Axis2::X)) => {
                &self.horizontal_dimension_border_columns
            }
            Border::Dimension(RowColBorder(HeadingRegion::Columns, Axis2::Y)) => {
                &self.vertical_category_border_columns
            }
            Border::Dimension(RowColBorder(HeadingRegion::Rows, Axis2::X)) => {
                &self.horizontal_dimension_border_rows
            }
            Border::Dimension(RowColBorder(HeadingRegion::Rows, Axis2::Y)) => {
                &self.vertical_category_border_rows
            }
            Border::Category(RowColBorder(HeadingRegion::Columns, Axis2::X)) => {
                &self.horizontal_category_border_columns
            }
            Border::Category(RowColBorder(HeadingRegion::Columns, Axis2::Y)) => {
                &self.vertical_category_border_columns
            }
            Border::Category(RowColBorder(HeadingRegion::Rows, Axis2::X)) => {
                &self.horizontal_category_border_rows
            }
            Border::Category(RowColBorder(HeadingRegion::Rows, Axis2::Y)) => {
                &self.vertical_category_border_rows
            }
            Border::DataLeft => &self.data_area_left,
            Border::DataTop => &self.data_area_top,
        }
    }

    fn decode(&self) -> EnumMap<Border, look::BorderStyle> {
        EnumMap::from_fn(|border: Border| {
            let mut base = Self::default_border_style(border);
            if let Some(style) = self.get_style(border) {
                if let Some(stroke) = style.stroke {
                    base.stroke = stroke;
                }
                if let Some(color) = style.color {
                    base.color = color;
                }
            }
            base
        })
    }

    /// Returns the default border style for `border` when reading an XML
    /// [Look].  (This is different from the system default.)
    pub fn default_border_style(border: Border) -> look::BorderStyle {
        const VERY_DARK_BLUE: Color = Color::new(0x15, 0x29, 0x35);
        const LIGHT_GRAY: Color = Color::new(0xe0, 0xe0, 0xe0);
        const GRAY: Color = Color::new(0xae, 0xae, 0xae);

        match border {
            Border::InnerFrame(BoxBorder::Bottom) | Border::DataTop => {
                look::BorderStyle::solid().with_color(VERY_DARK_BLUE)
            }
            Border::InnerFrame(_) | Border::OuterFrame(_) | Border::Title => {
                look::BorderStyle::none().with_color(VERY_DARK_BLUE)
            }
            Border::Dimension(_) => look::BorderStyle::none().with_color(GRAY),
            Border::Category(RowColBorder(HeadingRegion::Rows, Axis2::X)) => {
                look::BorderStyle::solid().with_color(GRAY)
            }
            Border::Category(RowColBorder(HeadingRegion::Columns, Axis2::Y)) => {
                look::BorderStyle::solid().with_color(LIGHT_GRAY)
            }
            Border::Category(_) => look::BorderStyle::none().with_color(GRAY),
            Border::DataLeft => look::BorderStyle::none().with_color(VERY_DARK_BLUE),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct BorderStyle {
    /// The kind of line to draw.
    #[serde(rename = "@borderStyleType")]
    pub stroke: Option<Stroke>,

    /// Line color.
    #[serde(rename = "@color")]
    pub color: Option<Color>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PrintingProperties {
    #[serde(rename = "@printAllLayers")]
    print_all_layers: bool,

    #[serde(rename = "@printEachLayerOnSeparatePage")]
    print_each_layer_on_separate_page: bool,

    #[serde(rename = "@rescaleWideTableToFitPage")]
    rescale_wide_table_to_fit_page: bool,

    #[serde(rename = "@rescaleLongTableToFitPage")]
    rescale_long_table_to_fit_page: bool,

    #[serde(rename = "@windowOrphanLines")]
    window_orphan_lines: i64,

    #[serde(rename = "@continuationText")]
    continuation_text: String,

    #[serde(rename = "@continuationTextAtBottom")]
    continuation_text_at_bottom: bool,

    #[serde(rename = "@continuationTextAtTop")]
    continuation_text_at_top: bool,
}

impl Default for PrintingProperties {
    fn default() -> Self {
        Self {
            print_all_layers: false,
            print_each_layer_on_separate_page: false,
            rescale_wide_table_to_fit_page: false,
            rescale_long_table_to_fit_page: false,
            window_orphan_lines: 2,
            continuation_text: String::from("(cont.)"),
            continuation_text_at_bottom: false,
            continuation_text_at_top: false,
        }
    }
}

/// A length.
#[derive(Copy, Clone, Default, PartialEq)]
pub struct Length(
    /// In inches.
    pub f64,
);

impl Length {
    /// Returns the length in 1/96" units.
    pub fn as_px_f64(self) -> f64 {
        self.0 * 96.0
    }
    /// Returns the length in 1/96" units.
    pub fn as_px_i32(self) -> i32 {
        num::cast(self.as_px_f64() + 0.5).unwrap_or_default()
    }
    /// Returns the length in 1/96" units.
    pub fn as_px_isize(self) -> isize {
        num::cast(self.as_px_f64() + 0.5).unwrap_or_default()
    }
    /// Returns the length in 1/72" units.
    pub fn as_pt_f64(self) -> f64 {
        self.0 * 72.0
    }
    /// Returns the length in 1/72" units.
    pub fn as_pt_i32(self) -> i32 {
        num::cast(self.as_pt_f64() + 0.5).unwrap_or_default()
    }
}

impl From<Length> for paper_sizes::Length {
    fn from(value: Length) -> Self {
        Self::new(value.0, paper_sizes::Unit::Inch)
    }
}

impl Debug for Length {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}in", self.0)
    }
}

impl FromStr for Length {
    type Err = LengthParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim_start();
        let unit = s.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ',');
        let value = &s[..s.len() - unit.len()];
        let number: f64 = if value.contains(',') {
            value.replace(',', ".").parse()
        } else {
            value.parse()
        }
        .map_err(LengthParseError::ParseFloatError)?;
        let divisor = match unit.trim() {
            // Inches.
            "in" | "인치" | "pol." | "cala" | "cali" => 1.0,

            // Device-independent pixels.
            "px" => 96.0,

            // Points.
            "pt" | "пт" | "" => 72.0,

            // Centimeters.
            "cm" | "см" => 2.54,

            other => return Err(LengthParseError::InvalidUnit(other.into())),
        };
        Ok(Length(number / divisor))
    }
}

#[derive(ThisError, Debug, PartialEq, Eq)]
pub enum LengthParseError {
    /// Invalid number.
    #[error(transparent)]
    ParseFloatError(ParseFloatError),

    /// Unknown unit.
    #[error("Unknown unit {0:?}")]
    InvalidUnit(String),
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct DimensionVisitor;

        impl<'de> Visitor<'de> for DimensionVisitor {
            type Value = Length;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a dimension expressed as a string, e.g. \"1.0 cm\"")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                v.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_str(DimensionVisitor)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use enum_map::{EnumMap, enum_map};
    use quick_xml::de::from_str;

    use crate::output::pivot::{
        Axis2, FootnoteMarkerPosition, FootnoteMarkerType,
        look::{
            Area, AreaStyle, Border, BorderStyle, BoxBorder, CellStyle, Color, FontStyle,
            HeadingRegion, HorzAlign, LabelPosition, Look, RowColBorder, RowParity, Stroke,
            VertAlign,
        },
        look_xml::{Length, LengthParseError, TableProperties},
    };

    #[test]
    fn dimension() {
        assert_eq!(Length::from_str("1"), Ok(Length(1.0 / 72.0)));
        assert_eq!(Length::from_str("1pt"), Ok(Length(1.0 / 72.0)));
        assert_eq!(Length::from_str("1пт"), Ok(Length(1.0 / 72.0)));
        assert_eq!(Length::from_str("1.0"), Ok(Length(1.0 / 72.0)));
        assert_eq!(Length::from_str(" 1.0"), Ok(Length(1.0 / 72.0)));
        assert_eq!(Length::from_str(" 1.0 "), Ok(Length(1.0 / 72.0)));
        assert_eq!(Length::from_str("1.0 pt"), Ok(Length(1.0 / 72.0)));
        assert_eq!(Length::from_str("1.0pt "), Ok(Length(1.0 / 72.0)));
        assert_eq!(Length::from_str(" 1.0pt "), Ok(Length(1.0 / 72.0)));

        assert_eq!(Length::from_str("1in"), Ok(Length(1.0)));

        assert_eq!(Length::from_str("96px"), Ok(Length(1.0)));

        assert_eq!(Length::from_str("2.54cm"), Ok(Length(1.0)));

        assert_eq!(
            Length::from_str(""),
            Err(LengthParseError::ParseFloatError(
                "".parse::<f64>().unwrap_err()
            ))
        );
        assert_eq!(
            Length::from_str("1.2.3"),
            Err(LengthParseError::ParseFloatError(
                "1.2.3".parse::<f64>().unwrap_err()
            ))
        );
        assert_eq!(
            Length::from_str("1asdf"),
            Err(LengthParseError::InvalidUnit("asdf".into()))
        );
    }

    #[test]
    fn look() {
        const XML: &str = r##"
<?xml version="1.0" encoding="UTF-8"?>
<tableProperties xmlns="http://www.ibm.com/software/analytics/spss/xml/table-looks" xmlns:vizml="http://www.ibm.com/software/analytics/spss/xml/visualization" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.ibm.com/software/analytics/spss/xml/table-looks http://www.ibm.com/software/analytics/spss/xml/table-looks/table-looks-1.4.xsd">
    <generalProperties hideEmptyRows="true" maximumColumnWidth="72" maximumRowWidth="120" minimumColumnWidth="36" minimumRowWidth="36" rowDimensionLabels="inCorner"/>
    <footnoteProperties markerPosition="subscript" numberFormat="alphabetic"/>
    <cellFormatProperties>
        <title>
            <vizml:style color="#000000" color2="#ffffff" font-family="Sans Serif" font-size="9pt" font-weight="bold" font-underline="none" labelLocationVertical="center" margin-bottom="6pt" margin-left="6pt" margin-right="8pt" margin-top="0pt" textAlignment="left"/>
        </title>
        <caption>
            <vizml:style color="#000000" color2="#ffffff" font-family="Sans Serif" font-size="9pt" font-weight="regular" font-underline="none" labelLocationVertical="positive" margin-bottom="0pt" margin-left="6pt" margin-right="8pt" margin-top="0pt" textAlignment="left"/>
        </caption>
        <footnotes>
            <vizml:style color="#000000" color2="#ffffff" font-family="Sans Serif" font-size="9pt" font-weight="regular" font-underline="none" labelLocationVertical="positive" margin-bottom="2pt" margin-left="8pt" margin-right="6pt" margin-top="1pt" textAlignment="left"/>
        </footnotes>
        <cornerLabels>
            <vizml:style color="#000000" color2="#ffffff" font-family="Sans Serif" font-size="9pt" font-weight="regular" font-underline="none" labelLocationVertical="negative" margin-bottom="0pt" margin-left="6pt" margin-right="8pt" margin-top="0pt" textAlignment="left"/>
        </cornerLabels>
        <columnLabels>
            <vizml:style color="#000000" color2="#ffffff" font-family="Sans Serif" font-size="9pt" font-weight="regular" font-underline="none" labelLocationVertical="negative" margin-bottom="2pt" margin-left="6pt" margin-right="8pt" margin-top="0pt" textAlignment="center"/>
        </columnLabels>
        <rowLabels>
            <vizml:style color="#000000" color2="#ffffff" font-family="Sans Serif" font-size="9pt" font-weight="regular" font-underline="none" labelLocationVertical="positive" margin-bottom="2pt" margin-left="6pt" margin-right="8pt" margin-top="0pt" textAlignment="left"/>
        </rowLabels>
        <data>
            <vizml:style color="#000000" color2="#ffffff" font-family="Sans Serif" font-size="9pt" font-weight="regular" font-underline="none" labelLocationVertical="positive" margin-bottom="0pt" margin-left="6pt" margin-right="8pt" margin-top="0pt" textAlignment="mixed"/>
        </data>
        <layers>
            <vizml:style color="#000000" color2="#ffffff" font-family="Sans Serif" font-size="9pt" font-weight="regular" font-underline="none" labelLocationVertical="negative" margin-bottom="2pt" margin-left="6pt" margin-right="8pt" margin-top="0pt" textAlignment="left"/>
        </layers>
    </cellFormatProperties>
    <borderProperties>
        <titleLayerSeparator borderStyleType="none" color="#000000"/>
        <leftOuterFrame borderStyleType="none" color="#000000"/>
        <topOuterFrame borderStyleType="none" color="#000000"/>
        <rightOuterFrame borderStyleType="none" color="#000000"/>
        <bottomOuterFrame borderStyleType="none" color="#000000"/>
        <leftInnerFrame borderStyleType="thick" color="#000000"/>
        <topInnerFrame borderStyleType="thick" color="#000000"/>
        <rightInnerFrame borderStyleType="thick" color="#000000"/>
        <bottomInnerFrame borderStyleType="thick" color="#000000"/>
        <dataAreaLeft borderStyleType="thick" color="#000000"/>
        <dataAreaTop borderStyleType="thick" color="#000000"/>
        <horizontalDimensionBorderRows borderStyleType="solid" color="#000000"/>
        <verticalDimensionBorderRows borderStyleType="none" color="#000000"/>
        <horizontalDimensionBorderColumns borderStyleType="solid" color="#000000"/>
        <verticalDimensionBorderColumns borderStyleType="solid" color="#000000"/>
        <horizontalCategoryBorderRows borderStyleType="none" color="#000000"/>
        <verticalCategoryBorderRows borderStyleType="none" color="#000000"/>
        <horizontalCategoryBorderColumns borderStyleType="solid" color="#000000"/>
        <verticalCategoryBorderColumns borderStyleType="solid" color="#000000"/>
    </borderProperties>
    <printingProperties printAllLayers="true" rescaleLongTableToFitPage="false" rescaleWideTableToFitPage="false" windowOrphanLines="5"/>
</tableProperties>
"##;
        let table_properties: TableProperties = from_str(XML).unwrap();
        let look: Look = table_properties.into();
        dbg!(&look);
        let expected = Look {
            name: None,
            hide_empty: true,
            row_label_position: LabelPosition::Corner,
            heading_widths: enum_map! {
                HeadingRegion::Rows => 48..=160,
                HeadingRegion::Columns => 48..=96,
            },
            footnote_marker_type: FootnoteMarkerType::Alphabetic,
            footnote_marker_position: FootnoteMarkerPosition::Subscript,
            areas: enum_map! {
                Area::Title => AreaStyle {
                    cell_style: CellStyle {
                        horz_align: Some(
                            HorzAlign::Left,
                        ),
                        vert_align: VertAlign::Middle,
                        margins: enum_map! {
                            Axis2::X => [
                                8,
                                11,
                            ],
                            Axis2::Y => [
                                0,
                                8,
                            ],
                        },
                    },
                    font_style: FontStyle {
                        bold: true,
                        italic: false,
                        underline: false,
                        font: String::from("Sans Serif"),
                        fg: Color::BLACK,
                        bg: Color::WHITE,
                        size: 9,
                    },
                },
                Area::Caption => AreaStyle {
                    cell_style: CellStyle {
                        horz_align: Some(
                            HorzAlign::Left,
                        ),
                        vert_align: VertAlign::Top,
                        margins: enum_map! {
                            Axis2::X => [
                                8,
                                11,
                            ],
                            Axis2::Y => [
                                0,
                                0,
                            ],
                        },
                    },
                    font_style: FontStyle {
                        bold: false,
                        italic: false,
                        underline: false,
                        font: String::from("Sans Serif"),
                        fg: Color::BLACK,
                        bg: Color::WHITE,
                        size: 9,
                    },
                },
                Area::Footer => AreaStyle {
                    cell_style: CellStyle {
                        horz_align: Some(
                            HorzAlign::Left,
                        ),
                        vert_align: VertAlign::Top,
                        margins: enum_map! {
                            Axis2::X => [
                                11,
                                8,
                            ],
                            Axis2::Y => [
                                1,
                                3,
                            ],
                        },
                    },
                    font_style: FontStyle {
                        bold: false,
                        italic: false,
                        underline: false,
                        font: String::from("Sans Serif"),
                        fg: Color::BLACK,
                        bg: Color::WHITE,
                        size: 9,
                    },
                },
                Area::Corner => AreaStyle {
                    cell_style: CellStyle {
                        horz_align: Some(
                            HorzAlign::Left,
                        ),
                        vert_align: VertAlign::Bottom,
                        margins: enum_map! {
                            Axis2::X => [
                                8,
                                11,
                            ],
                            Axis2::Y => [
                                0,
                                0,
                            ],
                        },
                    },
                    font_style: FontStyle {
                        bold: false,
                        italic: false,
                        underline: false,
                        font: String::from("Sans Serif"),
                        fg: Color::BLACK,
                        bg: Color::WHITE,
                        size: 9,
                    },
                },
                Area::Labels(
                    Axis2::X,
                ) => AreaStyle {
                    cell_style: CellStyle {
                        horz_align: Some(
                            HorzAlign::Center,
                        ),
                        vert_align: VertAlign::Bottom,
                        margins: enum_map! {
                            Axis2::X => [
                                8,
                                11,
                            ],
                            Axis2::Y => [
                                0,
                                3,
                            ],
                        },
                    },
                    font_style: FontStyle {
                        bold: false,
                        italic: false,
                        underline: false,
                        font: String::from("Sans Serif"),
                        fg: Color::BLACK,
                        bg: Color::WHITE,
                        size: 9,
                    },
                },
                Area::Labels(
                    Axis2::Y,
                )=> AreaStyle {
                    cell_style: CellStyle {
                        horz_align: Some(
                            HorzAlign::Left,
                        ),
                        vert_align: VertAlign::Top,
                        margins: enum_map! {
                            Axis2::X => [
                                8,
                                11,
                            ],
                            Axis2::Y => [
                                0,
                                3,
                            ],
                        },
                    },
                    font_style: FontStyle {
                        bold: false,
                        italic: false,
                        underline: false,
                        font: String::from("Sans Serif"),
                        fg: Color::BLACK,
                        bg: Color::WHITE,
                        size: 9,
                    },
                },
                Area::Data(
                    RowParity::Even,
                ) => AreaStyle {
                    cell_style: CellStyle {
                        horz_align: None,
                        vert_align: VertAlign::Top,
                        margins: enum_map! {
                            Axis2::X => [
                                8,
                                11,
                            ],
                            Axis2::Y => [
                                0,
                                0,
                            ],
                        },
                    },
                    font_style: FontStyle {
                        bold: false,
                        italic: false,
                        underline: false,
                        font: String::from("Sans Serif"),
                        fg: Color::BLACK,
                        bg: Color::WHITE,
                        size: 9,
                    },
                },
                Area::Data(
                    RowParity::Odd,
                )=>AreaStyle {
                    cell_style: CellStyle {
                        horz_align: None,
                        vert_align: VertAlign::Top,
                        margins: enum_map! {
                            Axis2::X => [
                                8,
                                11,
                            ],
                            Axis2::Y => [
                                0,
                                0,
                            ],
                        },
                    },
                    font_style: FontStyle {
                        bold: false,
                        italic: false,
                        underline: false,
                        font: String::from("Sans Serif"),
                        fg: Color::BLACK,
                        bg: Color::WHITE,
                        size: 9,
                    },
                },
                Area::Layers => AreaStyle {
                    cell_style: CellStyle {
                        horz_align: Some(
                            HorzAlign::Left,
                        ),
                        vert_align: VertAlign::Bottom,
                        margins: enum_map! {
                            Axis2::X => [
                                8,
                                11,
                            ],
                            Axis2::Y => [
                                0,
                                3,
                            ],
                        },
                    },
                    font_style: FontStyle {
                        bold: false,
                        italic: false,
                        underline: false,
                        font: String::from("Sans Serif"),
                        fg: Color::BLACK,
                        bg: Color::WHITE,
                        size: 9,
                    },
                },
            },
            borders: enum_map! {
                Border::Title => BorderStyle {
                    stroke: Stroke::None,
                    color: Color::BLACK,
                },
                Border::OuterFrame(
                    BoxBorder::Left,
                )=>BorderStyle {
                    stroke: Stroke::None,
                    color: Color::BLACK,
                },
                Border::OuterFrame(
                    BoxBorder::Top,
                ) =>BorderStyle {
                    stroke: Stroke::None,
                    color: Color::BLACK,
                },
                Border::OuterFrame(
                    BoxBorder::Right,
                ) => BorderStyle {
                    stroke: Stroke::None,
                    color: Color::BLACK,
                },
                Border::OuterFrame(
                    BoxBorder::Bottom,
                )=> BorderStyle {
                    stroke: Stroke::None,
                    color: Color::BLACK,
                },
                Border::InnerFrame(
                    BoxBorder::Left,
                )=> BorderStyle {
                    stroke: Stroke::Thick,
                    color: Color::BLACK,
                },
                Border::InnerFrame(
                    BoxBorder::Top,
                )=> BorderStyle {
                    stroke: Stroke::Thick,
                    color: Color::BLACK,
                },
                Border::InnerFrame(
                    BoxBorder::Right,
                )=> BorderStyle {
                    stroke: Stroke::Thick,
                    color: Color::BLACK,
                },
                Border::InnerFrame(
                    BoxBorder::Bottom,
                )=> BorderStyle {
                    stroke: Stroke::Thick,
                    color: Color::BLACK,
                },
                Border::Dimension(
                    RowColBorder(
                        HeadingRegion::Rows,
                        Axis2::X,
                    ),
                )=> BorderStyle {
                    stroke: Stroke::Solid,
                    color: Color::BLACK,
                },
                Border::Dimension(
                    RowColBorder(
                        HeadingRegion::Columns,
                        Axis2::X,
                    ),
                )=> BorderStyle {
                    stroke: Stroke::Solid,
                    color: Color::BLACK,
                },
                Border::Dimension(
                    RowColBorder(
                        HeadingRegion::Rows,
                        Axis2::Y,
                    ),
                )=> BorderStyle {
                    stroke: Stroke::None,
                    color: Color::BLACK,
                },
                Border::Dimension(
                    RowColBorder(
                        HeadingRegion::Columns,
                        Axis2::Y,
                    ),
                )=> BorderStyle {
                    stroke: Stroke::Solid,
                    color: Color::BLACK,
                },
                Border::Category(
                    RowColBorder(
                        HeadingRegion::Rows,
                        Axis2::X,
                    ),
                )=> BorderStyle {
                    stroke: Stroke::None,
                    color: Color::BLACK,
                },
                Border::Category(
                    RowColBorder(
                        HeadingRegion::Columns,
                        Axis2::X,
                    ),
                )=> BorderStyle {
                    stroke: Stroke::Solid,
                    color: Color::BLACK,
                },
                Border::Category(
                    RowColBorder(
                        HeadingRegion::Rows,
                        Axis2::Y,
                    ),
                )=> BorderStyle {
                    stroke: Stroke::None,
                    color: Color::BLACK,
                },
                Border::Category(
                    RowColBorder(
                        HeadingRegion::Columns,
                        Axis2::Y,
                    ),
                )=> BorderStyle {
                    stroke: Stroke::Solid,
                    color: Color::BLACK,
                },
                Border::DataLeft => BorderStyle {
                    stroke: Stroke::Thick,
                    color: Color::BLACK,
                },
                Border::DataTop => BorderStyle {
                    stroke: Stroke::Thick,
                    color: Color::BLACK,
                },
            },
            print_all_layers: true,
            paginate_layers: false,
            shrink_to_fit: EnumMap::from_fn(|_| false),
            show_continuations: [false, false],
            continuation: Some(String::from("(cont.)")),
            n_orphan_lines: 5,
        };
        assert_eq!(&look, &expected);
    }
}
