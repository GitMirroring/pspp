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

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::Arc,
};

use cairo::{Context, PdfSurface};
use chrono::{Local, NaiveDateTime};
use enum_map::{EnumMap, enum_map};
use pango::SCALE;
use paper_sizes::Unit;
use serde::{Deserialize, Serialize};

use crate::{
    output::{
        Item, TextType,
        drivers::{
            Driver,
            cairo::{
                fsm::{CairoFsmStyle, parse_font_style},
                pager::{CairoPageStyle, CairoPager},
            },
        },
        page::PageSetup,
        pivot::{
            Coord2,
            look::{Color, FontStyle},
        },
    },
    spv::html::Variable,
};

use crate::output::pivot::Axis2;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CairoConfig {
    /// Output file name.
    pub file: PathBuf,

    /// Page setup.
    pub page_setup: Option<PageSetup>,
}

impl CairoConfig {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            file: path.as_ref().to_path_buf(),
            page_setup: None,
        }
    }
}

pub struct CairoDriver {
    now: NaiveDateTime,
    fsm_style: Arc<CairoFsmStyle>,
    page_style: Arc<CairoPageStyle>,
    pager: Option<CairoPager>,
    surface: PdfSurface,
    title: String,
}

impl CairoDriver {
    pub fn new(config: &CairoConfig) -> cairo::Result<Self> {
        fn scale(inches: f64) -> isize {
            (inches * 72.0 * SCALE as f64).max(0.0).round() as isize
        }

        let default_page_setup;
        let page_setup = match &config.page_setup {
            Some(page_setup) => page_setup,
            None => {
                default_page_setup = PageSetup::default();
                &default_page_setup
            }
        };
        let printable = page_setup.printable_size();
        let page_style = CairoPageStyle {
            margins: EnumMap::from_fn(|axis| {
                [
                    scale(page_setup.margins.0[axis][0].into_unit(Unit::Inch)),
                    scale(page_setup.margins.0[axis][1].into_unit(Unit::Inch)),
                ]
            }),
            header: page_setup.header.clone(),
            footer: page_setup.footer.clone(),
            initial_page_number: page_setup.initial_page_number,
        };
        let size = Coord2::new(scale(printable[Axis2::X]), scale(printable[Axis2::Y]));
        let font = FontStyle::default().with_size(9);
        let font = parse_font_style(&font);
        let fsm_style = CairoFsmStyle {
            size,
            min_break: enum_map! {
                Axis2::X => size[Axis2::X] / 2,
                Axis2::Y => size[Axis2::Y] / 2,
            },
            font,
            fg: Color::BLACK,
            use_system_colors: false,
            object_spacing: scale(page_setup.object_spacing.into_unit(Unit::Inch)),
            font_resolution: 72.0,
        };
        let (width, height) = page_setup.paper.as_unit(Unit::Point).into_width_height();
        let surface = PdfSurface::new(width, height, &config.file)?;
        Ok(Self {
            now: Local::now().naive_local(),
            fsm_style: Arc::new(fsm_style),
            page_style: Arc::new(page_style),
            pager: None,
            surface,
            title: String::new(),
        })
    }
}

impl Driver for CairoDriver {
    fn name(&self) -> Cow<'static, str> {
        Cow::from("cairo")
    }

    fn write(&mut self, item: &Arc<Item>) {
        let pager = self.pager.get_or_insert_with(|| {
            CairoPager::new(self.page_style.clone(), self.fsm_style.clone())
        });
        let mut cursor = item.clone().cursor();
        while let Some(item) = cursor.cur() {
            if let Some(text) = item.details.as_text()
                && text.type_ == TextType::PageTitle
            {
                self.title = text.content.display(()).to_string();
            } else {
                pager.add_item(item.clone());
                while pager.needs_new_page() {
                    let context = Context::new(&self.surface).unwrap();
                    if pager.has_page() {
                        pager.finish_page();
                        context.show_page().unwrap();
                    }

                    pager.add_page(context, |variable, page_number| match variable {
                        Variable::Date => self.now.format("%Y-%m-%d").to_string(),
                        Variable::Time => self.now.format("%H:%M:%S").to_string(),
                        Variable::Head(level) => {
                            cursor.heading(level as usize).unwrap_or_default().into()
                        }
                        Variable::PageTitle => self.title.clone(),
                        Variable::Page => page_number.to_string(),
                    });
                }
            }
            cursor.next();
        }
    }
}

impl Drop for CairoDriver {
    fn drop(&mut self) {
        if self.pager.is_some() {
            let context = Context::new(&self.surface).unwrap();
            context.show_page().unwrap();
        }
    }
}
