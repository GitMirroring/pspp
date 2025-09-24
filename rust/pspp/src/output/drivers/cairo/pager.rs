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

use std::{borrow::Cow, sync::Arc};

use cairo::{Context, RecordingSurface};
use enum_map::EnumMap;
use pango::Layout;

use crate::{
    output::{
        Item,
        drivers::cairo::{
            fsm::{CairoFsm, CairoFsmStyle},
            xr_to_pt,
        },
        pivot::{
            Axis2, Rect2,
            look::{CellStyle, FontStyle},
            value::ValueOptions,
        },
        table::DrawCell,
    },
    spv::html::{Document, Variable},
};

#[derive(Clone, Debug)]
pub struct CairoPageStyle {
    pub margins: EnumMap<Axis2, [isize; 2]>,
    pub header: Document,
    pub footer: Document,
    pub initial_page_number: i32,
}

pub struct CairoPager {
    page_style: Arc<CairoPageStyle>,
    fsm_style: Arc<CairoFsmStyle>,
    page_index: i32,
    item: Option<Arc<Item>>,
    context: Option<Context>,
    fsm: Option<CairoFsm>,
    y: isize,
    y_max: isize,
}

impl CairoPager {
    pub fn new(page_style: Arc<CairoPageStyle>, fsm_style: Arc<CairoFsmStyle>) -> Self {
        Self {
            page_style,
            fsm_style,
            page_index: 0,
            item: None,
            context: None,
            fsm: None,
            y: 0,
            y_max: 0,
        }
    }

    pub fn has_page(&self) -> bool {
        self.context.is_some()
    }

    pub fn add_page<F>(&mut self, context: Context, substitutions: F)
    where
        F: Fn(Variable, i32) -> String,
    {
        assert!(self.context.is_none());
        context.save().unwrap();

        context.translate(
            xr_to_pt(self.page_style.margins[Axis2::X][0]),
            xr_to_pt(self.page_style.margins[Axis2::Y][0]),
        );

        let page_number = self.page_index + self.page_style.initial_page_number;
        self.page_index += 1;

        // Render header.
        let header = RenderHeading {
            fsm_style: &self.fsm_style,
            heading: &self.page_style.header,
            width: self.fsm_style.size[Axis2::X],
            font_resolution: self.fsm_style.font_resolution,
            page_number,
            substitutions,
        };
        self.y = header.render(&context, 0);

        // Render footer.
        let footer = header.with_heading(&self.page_style.footer);
        let footer_size = footer.measure();
        self.y_max = self.fsm_style.size[Axis2::Y] - footer_size;
        if footer_size > 0 {
            footer.render(&context, self.y_max);
        }

        context.translate(0.0, xr_to_pt(self.y));

        self.context = Some(context);
        self.run();
    }

    pub fn finish_page(&mut self) {
        if let Some(context) = self.context.take() {
            context.restore().unwrap();
        }
    }

    pub fn needs_new_page(&self) -> bool {
        (self.item.is_some() || self.fsm.is_some())
            && (self.context.is_none() || self.y >= self.y_max)
    }

    pub fn add_item(&mut self, item: Arc<Item>) {
        self.item = Some(item);
        self.run();
    }

    fn run(&mut self) {
        if self.needs_new_page() {
            return;
        }
        let Some(context) = self.context.as_ref().cloned() else {
            return;
        };

        loop {
            // Make sure we've got an object to render.
            let fsm = match &mut self.fsm {
                Some(fsm) => fsm,
                None => {
                    let Some(item) = self.item.take() else {
                        return;
                    };
                    self.fsm
                        .insert(CairoFsm::new(self.fsm_style.clone(), true, &context, item))
                }
            };

            // Prepare to render the current object.
            let chunk = fsm.draw_slice(&context, (self.y_max - self.y).max(0));
            self.y += chunk + self.fsm_style.object_spacing;
            context.translate(0.0, xr_to_pt(chunk + self.fsm_style.object_spacing));

            if fsm.is_done() {
                self.fsm = None;
            } else if chunk == 0 {
                assert!(self.y > 0);
                self.y = isize::MAX;
                return;
            }
        }
    }
}

struct RenderHeading<'a, F> {
    heading: &'a Document,
    fsm_style: &'a CairoFsmStyle,
    page_number: i32,
    width: isize,
    font_resolution: f64,
    substitutions: F,
}

impl<'a, F> RenderHeading<'a, F>
where
    F: Fn(Variable, i32) -> String,
{
    fn with_heading(self, heading: &'a Document) -> Self {
        Self { heading, ..self }
    }

    fn measure(&self) -> isize {
        let surface = RecordingSurface::create(cairo::Content::Color, None).unwrap();
        let context = Context::new(&surface).unwrap();
        self.render(&context, 0)
    }

    fn render(&self, context: &Context, base_y: isize) -> isize {
        let pangocairo_context = pangocairo::functions::create_context(context);
        pangocairo::functions::context_set_resolution(&pangocairo_context, self.font_resolution);

        let mut y = 0;
        let default_cell_style = CellStyle::default();
        let default_font_style = FontStyle::default();
        let value_options = ValueOptions::default();
        let substitutions =
            &|variable| Some(Cow::from((self.substitutions)(variable, self.page_number)));

        for block in self.heading.to_values() {
            // XXX substitute heading variables
            let cell = DrawCell {
                rotate: false,
                inner: &block.inner,
                cell_style: block.cell_style().unwrap_or(&default_cell_style),
                font_style: block.font_style().unwrap_or(&default_font_style),
                subscripts: block.subscripts(),
                footnotes: block.footnotes(),
                value_options: &value_options,
                substitutions,
            };
            let mut layout = Layout::new(&pangocairo_context);
            let bb = Rect2::new(0..self.width, y + base_y..isize::MAX);
            cell.layout(&bb, &mut layout, &self.fsm_style.font);
            cell.draw(&bb, &layout, None, context);
            y += layout.size().1 as isize;
        }
        if y > 0 {
            y + self.fsm_style.object_spacing
        } else {
            0
        }
    }
}
