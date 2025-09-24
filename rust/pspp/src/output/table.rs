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

//! Tables.
//!
//! A table is a rectangular grid of cells.  Cells can be joined to form larger
//! cells.  Rows and columns can be separated by rules of various types.  Rows
//! at the top and bottom of a table and columns at the left and right edges of
//! a table can be designated as headers, which means that if the table must be
//! broken across more than one page, those rows or columns are repeated on each
//! page.
//!
//! Some drivers use tables as an implementation detail of rendering pivot
//! tables.

use std::{
    borrow::Cow,
    ops::{Index, IndexMut, Range},
    sync::Arc,
};

use enum_map::{EnumMap, enum_map};
use ndarray::{Array, Array2};

use crate::{
    output::pivot::{
        Axis2, Footnote,
        look::{
            Area, AreaStyle, Border, BorderStyle, CellStyle, FontStyle, HeadingRegion, HorzAlign,
            RowParity,
        },
        value::{DisplayValue, Value, ValueInner, ValueOptions},
    },
    spv::html,
};

/// The `(x,y)` position of a cell in a [Table].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellPos {
    /// X
    pub x: usize,
    /// Y
    pub y: usize,
}

impl CellPos {
    pub fn new(x: usize, y: usize) -> Self {
        Self { x, y }
    }
    pub fn for_axis((a, az): (Axis2, usize), bz: usize) -> Self {
        match a {
            Axis2::X => Self::new(az, bz),
            Axis2::Y => Self::new(bz, az),
        }
    }

    pub fn from_fn<F>(mut f: F) -> Self
    where
        F: FnMut(Axis2) -> usize,
    {
        Self::new(f(Axis2::X), f(Axis2::Y))
    }
}

impl Index<Axis2> for CellPos {
    type Output = usize;

    fn index(&self, index: Axis2) -> &Self::Output {
        match index {
            Axis2::X => &self.x,
            Axis2::Y => &self.y,
        }
    }
}

impl IndexMut<Axis2> for CellPos {
    fn index_mut(&mut self, index: Axis2) -> &mut Self::Output {
        match index {
            Axis2::X => &mut self.x,
            Axis2::Y => &mut self.y,
        }
    }
}

/// A rectangular group of cells in a [Table].
#[derive(Clone, Debug, Default)]
pub struct CellRect {
    /// X range.
    pub x: Range<usize>,
    /// Y range.
    pub y: Range<usize>,
}

impl CellRect {
    pub fn new(x: Range<usize>, y: Range<usize>) -> Self {
        Self { x, y }
    }
    pub fn for_cell(cell: CellPos) -> Self {
        Self::new(cell.x..cell.x + 1, cell.y..cell.y + 1)
    }
    pub fn for_ranges((a_axis, a): (Axis2, Range<usize>), b: Range<usize>) -> Self {
        match a_axis {
            Axis2::X => Self { x: a, y: b },
            Axis2::Y => Self { x: b, y: a },
        }
    }
    pub fn top_left(&self) -> CellPos {
        CellPos::new(self.x.start, self.y.start)
    }
    pub fn is_empty(&self) -> bool {
        self.x.is_empty() || self.y.is_empty()
    }
    pub fn map<F>(&self, mut f: F) -> Self
    where
        F: FnMut(Axis2, Range<usize>) -> Range<usize>,
    {
        Self {
            x: f(Axis2::X, self.x.clone()),
            y: f(Axis2::Y, self.y.clone()),
        }
    }
}

impl Index<Axis2> for CellRect {
    type Output = Range<usize>;

    fn index(&self, index: Axis2) -> &Self::Output {
        match index {
            Axis2::X => &self.x,
            Axis2::Y => &self.y,
        }
    }
}

impl IndexMut<Axis2> for CellRect {
    fn index_mut(&mut self, index: Axis2) -> &mut Self::Output {
        match index {
            Axis2::X => &mut self.x,
            Axis2::Y => &mut self.y,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CellRef<'a> {
    pub pos: CellPos,
    pub content: &'a Content,
}

impl CellRef<'_> {
    pub fn inner(&self) -> &CellInner {
        self.content.inner()
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    pub fn rect(&self) -> CellRect {
        self.content.rect(self.pos)
    }

    pub fn next_x(&self) -> usize {
        self.content.next_x(self.pos.x)
    }

    pub fn is_top_left(&self) -> bool {
        self.content.is_top_left(self.pos)
    }

    pub fn span(&self, axis: Axis2) -> usize {
        self.content.span(axis)
    }
    pub fn col_span(&self) -> usize {
        self.span(Axis2::X)
    }
    pub fn row_span(&self) -> usize {
        self.span(Axis2::Y)
    }
}

#[derive(Clone, Debug)]
pub enum Content {
    Value(CellInner),
    Join(Arc<Cell>),
}

impl Content {
    pub fn inner(&self) -> &CellInner {
        match self {
            Content::Value(cell_inner) => cell_inner,
            Content::Join(cell) => &cell.inner,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inner().is_empty()
    }

    /// Returns the rectangle that this cell covers, only if the cell contains
    /// that information. (Joined cells always do, and other cells usually
    /// don't.)
    pub fn joined_rect(&self) -> Option<&CellRect> {
        match self {
            Content::Join(cell) => Some(&cell.region),
            _ => None,
        }
    }

    /// Returns the rectangle that this cell covers. If the cell doesn't contain
    /// that information, returns a rectangle containing `pos`.
    pub fn rect(&self, pos: CellPos) -> CellRect {
        match self {
            Content::Join(cell) => cell.region.clone(),
            _ => CellRect::for_cell(pos),
        }
    }

    pub fn next_x(&self, x: usize) -> usize {
        self.joined_rect().map_or(x + 1, |region| region.x.end)
    }

    pub fn is_top_left(&self, pos: CellPos) -> bool {
        self.joined_rect().is_none_or(|r| pos == r.top_left())
    }

    pub fn span(&self, axis: Axis2) -> usize {
        self.joined_rect().map_or(1, |r| {
            let range = &r[axis];
            range.end - range.start
        })
    }

    pub fn col_span(&self) -> usize {
        self.span(Axis2::X)
    }
    pub fn row_span(&self) -> usize {
        self.span(Axis2::Y)
    }
    pub fn default_for_area(area: Area) -> Self {
        Self::Value(CellInner::new(area, Default::default()))
    }
}

#[derive(Clone, Debug)]
pub struct Cell {
    inner: CellInner,

    /// Occupied table region.
    region: CellRect,
}

impl Cell {
    fn new(inner: CellInner, region: CellRect) -> Self {
        Self { inner, region }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CellInner {
    /// Rotate cell contents 90 degrees?
    pub rotate: bool,

    /// The area that the cell belongs to.
    pub area: Area,

    pub value: Box<Value>,
}

impl CellInner {
    pub fn new(area: Area, value: Box<Value>) -> Self {
        Self {
            rotate: false,
            area,
            value,
        }
    }

    pub fn with_rotate(self, rotate: bool) -> Self {
        Self { rotate, ..self }
    }

    pub fn is_empty(&self) -> bool {
        self.value.inner.is_empty()
    }
}

/// A table.
#[derive(derive_more::Debug)]
pub struct Table {
    /// Number of rows and columns.
    pub n: CellPos,

    /// Table header rows and columns.
    ///
    /// This is a subset of `n`, so `h.x <= n.x` and
    /// `h.y <= n.y`.
    pub h: CellPos,

    /// Table contents.
    ///
    /// The array has `n.x` columns and `n.y` rows.
    pub contents: Array2<Content>,

    /// Styles for areas of the table.
    #[debug(skip)]
    pub areas: EnumMap<Area, AreaStyle>,

    /// Styles for borders in the table.
    #[debug(skip)]
    pub borders: EnumMap<Border, BorderStyle>,

    /// Horizontal ([Axis2::Y]) and vertical ([Axis2::X]) rules.
    pub rules: EnumMap<Axis2, Array2<Option<Border>>>,

    /// How to present values.
    #[debug(skip)]
    pub value_options: ValueOptions,
}

impl Table {
    pub fn new(
        n: CellPos,
        headers: CellPos,
        areas: EnumMap<Area, AreaStyle>,
        borders: EnumMap<Border, BorderStyle>,
        value_options: impl Into<ValueOptions>,
    ) -> Self {
        Self {
            n,
            h: headers,
            contents: Array::from_shape_fn((n.x, n.y), |(x, y)| {
                let area = match (x < headers.x, y < headers.y) {
                    (true, true) => Area::Corner,
                    (true, false) => Area::Labels(Axis2::Y),
                    (false, true) => Area::Labels(Axis2::X),
                    (false, false) => Area::Data(RowParity::Even),
                };
                Content::default_for_area(area)
            }),
            areas,
            borders,
            rules: enum_map! {
                Axis2::X => Array::default((n.x + 1, n.y)),
                Axis2::Y => Array::default((n.x, n.y + 1)),
            },
            value_options: value_options.into(),
        }
    }

    pub fn get(&self, coord: CellPos) -> CellRef<'_> {
        CellRef {
            pos: coord,
            content: &self.contents[[coord.x, coord.y]],
        }
    }

    pub fn get_rule(&self, axis: Axis2, pos: CellPos) -> Option<BorderStyle> {
        self.rules[axis][[pos.x, pos.y]].map(|b| self.borders[b])
    }

    pub fn put(&mut self, region: CellRect, inner: CellInner) {
        if region.x.len() == 1 && region.y.len() == 1 {
            self.contents[[region.x.start, region.y.start]] = Content::Value(inner);
        } else {
            let cell = Arc::new(Cell::new(inner, region.clone()));
            for y in region.y.clone() {
                for x in region.x.clone() {
                    self.contents[[x, y]] = Content::Join(cell.clone())
                }
            }
        }
    }

    pub fn h_line(&mut self, border: Border, x: Range<usize>, y: usize) {
        for x in x {
            self.rules[Axis2::Y][[x, y]] = Some(border);
        }
    }

    pub fn v_line(&mut self, border: Border, x: usize, y: Range<usize>) {
        for y in y {
            self.rules[Axis2::X][[x, y]] = Some(border);
        }
    }

    pub fn draw_line(
        &mut self,
        border: Border,
        (a, a_value): (Axis2, usize),
        b_range: Range<usize>,
    ) {
        match a {
            Axis2::X => self.h_line(border, b_range, a_value),
            Axis2::Y => self.v_line(border, a_value, b_range),
        }
    }

    pub fn iter_x(&self, y: usize) -> XIter<'_> {
        XIter {
            table: self,
            x: None,
            y,
        }
    }

    /// The heading region that `pos` is part of, if any.
    pub fn heading_region(&self, pos: CellPos) -> Option<HeadingRegion> {
        if pos.x < self.h.x {
            Some(HeadingRegion::Rows)
        } else if pos.y < self.h.y {
            Some(HeadingRegion::Columns)
        } else {
            None
        }
    }

    /// Iterates across all of the cells in the table, visiting each of them
    /// once in top-down, left-to-right order. Spanned cells are visited once,
    /// at the point in the iteration where their top-left cell would appear if
    /// they were not spanned.
    pub fn cells(&self) -> Cells<'_> {
        Cells::new(self)
    }

    pub fn is_empty(&self) -> bool {
        self.n[Axis2::X] == 0 || self.n[Axis2::Y] == 0
    }
}

pub struct XIter<'a> {
    table: &'a Table,
    x: Option<usize>,
    y: usize,
}

impl Iterator for XIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        let next_x = self
            .x
            .map_or(0, |x| self.table.get(CellPos::new(x, self.y)).next_x());
        if next_x >= self.table.n.x {
            None
        } else {
            self.x = Some(next_x);
            Some(next_x)
        }
    }
}

/// Iterator for all of the cells in a table (see [Table::cells]).
pub struct Cells<'a> {
    table: &'a Table,
    next: Option<CellRef<'a>>,
}

impl<'a> Cells<'a> {
    fn new(table: &'a Table) -> Self {
        Self {
            table,
            next: if table.is_empty() {
                None
            } else {
                Some(table.get(CellPos::new(0, 0)))
            },
        }
    }
}

impl<'a> Iterator for Cells<'a> {
    type Item = CellRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        use Axis2::*;
        let this = self.next.as_ref()?.clone();

        let mut next = this.clone();
        self.next = loop {
            let next_x = next.next_x();
            let coord = if next_x < self.table.n[X] {
                CellPos::new(next_x, next.pos.y)
            } else if next.pos.y + 1 < self.table.n[Y] {
                CellPos::new(0, next.pos.y + 1)
            } else {
                break None;
            };
            next = self.table.get(coord);
            if next.is_top_left() {
                break Some(next);
            }
        };
        Some(this)
    }
}

pub struct DrawCell<'a, 'b> {
    pub rotate: bool,
    pub inner: &'a ValueInner,
    pub cell_style: &'a CellStyle,
    pub font_style: &'a FontStyle,
    pub subscripts: &'a [String],
    pub footnotes: &'a [Arc<Footnote>],
    pub value_options: &'a ValueOptions,
    pub substitutions: &'b dyn Fn(html::Variable) -> Option<Cow<'b, str>>,
}

impl<'a, 'b> DrawCell<'a, 'b> {
    pub fn new(inner: &'a CellInner, table: &'a Table) -> Self {
        Self {
            rotate: inner.rotate,
            inner: &inner.value.inner,
            font_style: inner
                .value
                .font_style()
                .unwrap_or(&table.areas[inner.area].font_style),
            cell_style: inner
                .value
                .cell_style()
                .unwrap_or(&table.areas[inner.area].cell_style),
            subscripts: inner.value.subscripts(),
            footnotes: inner.value.footnotes(),
            value_options: &table.value_options,
            substitutions: &|_| None,
        }
    }

    pub fn display(&self) -> DisplayValue<'a> {
        self.inner
            .display(self.value_options)
            .with_subscripts(self.subscripts)
            .with_footnotes(self.footnotes)
    }

    pub fn horz_align(&self, display: &DisplayValue) -> HorzAlign {
        self.cell_style
            .horz_align
            .unwrap_or_else(|| HorzAlign::for_mixed(display.var_type()))
    }
}
