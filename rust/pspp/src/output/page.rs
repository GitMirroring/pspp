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

use std::{str::FromStr, sync::LazyLock};

use enum_map::{EnumMap, enum_map};
use paper_sizes::{Catalog, Length, PaperSize, Unit};
use serde::{Deserialize, Deserializer, Serialize, de::Error};

use crate::spv::html::Document;

use super::pivot::Axis2;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// Chart size.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartSize {
    /// Size specified in the chart itself.
    #[default]
    AsIs,

    /// Full page.
    FullHeight,

    /// Half-page.
    HalfHeight,

    /// Quarter-page.
    QuarterHeight,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct PageSetup {
    /// Page number of first page.
    pub initial_page_number: i32,

    /// Paper size in inches.
    #[serde(deserialize_with = "deserialize_paper_size")]
    pub paper: PaperSize,

    /// Margin width.
    pub margins: Margins,

    /// Portrait or landscape.
    pub orientation: Orientation,

    /// Space between objects.
    pub object_spacing: Length,

    /// Size of charts.
    pub chart_size: ChartSize,

    /// Header.
    pub header: Document,

    /// Footer.
    pub footer: Document,
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| Catalog::new());

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Margins(pub EnumMap<Axis2, [Length; 2]>);

impl Margins {
    fn new(top: Length, right: Length, bottom: Length, left: Length) -> Self {
        Self(enum_map! {
            Axis2::X => [left, right],
            Axis2::Y => [top, bottom],
        })
    }

    fn new_uniform(width: Length) -> Self {
        Self(EnumMap::from_fn(|_| [width, width]))
    }

    fn new_width_height(width: Length, height: Length) -> Self {
        Self(enum_map! {
            Axis2::X => [width, width],
            Axis2::Y => [height, height],
        })
    }

    fn total(&self, axis: Axis2, unit: Unit) -> f64 {
        self.0[axis][0].into_unit(unit) + self.0[axis][1].into_unit(unit)
    }
}

impl Default for Margins {
    fn default() -> Self {
        Self::new_uniform(Length::new(0.5, Unit::Inch))
    }
}

impl Serialize for Margins {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        {
            let l = self.0[Axis2::X][0];
            let r = self.0[Axis2::X][1];
            let t = self.0[Axis2::Y][0];
            let b = self.0[Axis2::Y][1];
            if l == r {
                if t == b {
                    if l == t {
                        l.serialize(serializer)
                    } else {
                        [t, l].serialize(serializer)
                    }
                } else {
                    [t, l, b].serialize(serializer)
                }
            } else {
                [t, r, b, l].serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for Margins {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Margins {
            Array(Vec<Length>),
            Value(Length),
        }
        let (t, r, b, l) = match Margins::deserialize(deserializer)? {
            Margins::Array(items) if items.len() == 1 => (items[0], items[0], items[0], items[0]),
            Margins::Array(items) if items.len() == 2 => (items[0], items[1], items[0], items[1]),
            Margins::Array(items) if items.len() == 3 => (items[0], items[1], items[2], items[1]),
            Margins::Array(items) if items.len() == 4 => (items[0], items[1], items[2], items[3]),
            Margins::Value(value) => (value, value, value, value),
            _ => return Err(D::Error::custom("invalid margins")),
        };
        Ok(Self(enum_map! {
            Axis2::X => [l, r],
            Axis2::Y => [t, b],
        }))
    }
}

pub fn deserialize_paper_size<'de, D>(deserializer: D) -> Result<PaperSize, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    PaperSize::from_str(&s).or_else(|_| {
        CATALOG
            .get_by_name(&s)
            .map(|spec| spec.size)
            .ok_or_else(|| D::Error::custom("unknown or invalid paper size {size}"))
    })
}

fn paper_size_to_enum_map(paper_size: PaperSize) -> EnumMap<Axis2, f64> {
    let (w, h) = paper_size
        .as_unit(paper_sizes::Unit::Inch)
        .into_width_height();
    enum_map! {
        Axis2::X => w,
        Axis2::Y => h
    }
}

impl Default for PageSetup {
    fn default() -> Self {
        Self {
            initial_page_number: 1,
            paper: CATALOG.default_paper().size,
            margins: Margins::default(),
            orientation: Default::default(),
            object_spacing: Length::new(12.0, Unit::Point),
            chart_size: Default::default(),
            header: Document::from_html(r#"<p align="center">&[PageTitle]</p>"#),
            footer: Document::from_html(r#"<p align="right">Page &[Page]</p>"#),
        }
    }
}

impl PageSetup {
    pub fn printable_size(&self) -> EnumMap<Axis2, f64> {
        let paper = paper_size_to_enum_map(self.paper);
        EnumMap::from_fn(|axis| paper[axis] - self.margins.total(axis, Unit::Inch))
    }
}

#[cfg(test)]
mod tests {
    use paper_sizes::{Length, Unit};

    use crate::output::page::{Margins, PageSetup};

    #[test]
    fn margins() {
        let a = Length::new(1.0, Unit::Inch);
        let b = Length::new(2.0, Unit::Point);
        let c = Length::new(3.0, Unit::Millimeter);
        let d = Length::new(4.5, Unit::Millimeter);
        assert_eq!(
            serde_json::to_string(&Margins::new_uniform(a)).unwrap(),
            "\"1in\""
        );
        assert_eq!(
            serde_json::from_str::<Margins>("\"1in\"").unwrap(),
            Margins::new_uniform(a)
        );
        assert_eq!(
            serde_json::from_str::<Margins>("[\"1in\"]").unwrap(),
            Margins::new_uniform(a)
        );
        assert_eq!(
            serde_json::to_string(&Margins::new_width_height(a, b)).unwrap(),
            "[\"2pt\",\"1in\"]"
        );
        assert_eq!(
            serde_json::to_string(&Margins::new(a, b, c, b)).unwrap(),
            "[\"1in\",\"2pt\",\"3mm\"]"
        );
        assert_eq!(
            serde_json::to_string(&Margins::new(a, b, c, d)).unwrap(),
            "[\"1in\",\"2pt\",\"3mm\",\"4.5mm\"]"
        );
    }

    #[test]
    fn page_setup() {
        let s = toml::to_string(&PageSetup::default()).unwrap();
        assert_eq!(
            toml::from_str::<PageSetup>(&s).unwrap(),
            PageSetup::default()
        );
    }
}
