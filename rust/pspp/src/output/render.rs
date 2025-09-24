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

use std::cmp::{max, min};
use std::iter::{once, zip};
use std::ops::Range;
use std::sync::Arc;

use enum_map::{Enum, EnumMap, enum_map};
use itertools::{Itertools, interleave};
use num::Integer;
use smallvec::SmallVec;

use crate::output::pivot::look::Color;
use crate::output::{
    pivot::{
        Axis2, Coord2, PivotTable, Rect2,
        look::{BorderStyle, Look, Stroke, VertAlign},
    },
    table::{CellPos, CellRect, Content, DrawCell, Table},
};

/// Parameters for rendering a table_item to a device.
///
///
/// # Coordinate system
///
/// The rendering code assumes that larger `x` is to the right and larger `y`
/// toward the bottom of the page.
///
/// The rendering code assumes that the table being rendered has its upper left
/// corner at (0,0) in device coordinates.  This is usually not the case from
/// the driver's perspective, so the driver should expect to apply its own
/// offset to coordinates passed to callback functions.
pub struct Params {
    /// Page size to try to fit the rendering into.  Some tables will, of
    /// course, overflow this size.
    pub size: Coord2,

    /// Nominal size of a character in the most common font:
    /// `font_size[Axis2::X]` is the em width.
    /// `font_size[Axis2::Y]` is the line leading.
    pub font_size: EnumMap<Axis2, isize>,

    /// Width of different kinds of lines.
    pub line_widths: EnumMap<Stroke, isize>,

    /// 1/96" of an inch (1px) in the rendering unit.  Currently used only for
    /// column width ranges, as in `width_ranges` in [Look].  Set to `None` to
    /// disable this feature.
    ///
    /// [Look]: crate::output::pivot::look::Look
    pub px_size: Option<isize>,

    /// Minimum cell width or height before allowing the cell to be broken
    /// across two pages.  (Joined cells may always be broken at join
    /// points.)
    pub min_break: EnumMap<Axis2, isize>,

    /// True if the driver supports cell margins.  (If false, the rendering
    /// engine will insert a small space betweeen adjacent cells that don't have
    /// an intervening rule.)
    pub supports_margins: bool,

    /// True if the local language has a right-to-left direction, otherwise
    /// false.
    pub rtl: bool,

    /// True if the table is being rendered for printing (as opposed to
    /// on-screen display).
    pub printing: bool,

    /// Whether [Device::adjust_break] is implemented.
    pub can_adjust_break: bool,

    /// Whether [Device::scale] is implemented.
    pub can_scale: bool,
}

impl Params {
    /// Returns a small but visible width.
    fn em(&self) -> isize {
        self.font_size[Axis2::X]
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Enum)]
pub enum Extreme {
    Min,
    Max,
}

pub trait Device {
    fn params(&self) -> &Params;

    /// Measures `cell`'s width.  Returns `map`, where:
    ///
    /// - `map[Extreme::Min]` is the minimum width required to avoid splitting a
    ///   single word across multiple lines.  This is usually the width of the
    ///   longest word in the cell.
    ///
    /// - `map[Extreme::Max]` is the minimum width required to avoid line breaks
    ///   other than at new-lines.
    fn measure_cell_width(&self, cell: &DrawCell) -> EnumMap<Extreme, isize>;

    /// Returns the height required to render `cell` given a width of `width`.
    fn measure_cell_height(&self, cell: &DrawCell, width: isize) -> isize;

    /// Given that there is space measuring `size` to render `cell`, where
    /// `size.y()` is insufficient to render the entire height of the cell,
    /// returns the largest height less than `size.y()` at which it is
    /// appropriate to break the cell.  For example, if breaking at the
    /// specified `size.y()` would break in the middle of a line of text, the
    /// return value would be just sufficiently less that the breakpoint would
    /// be between lines of text.
    ///
    /// Optional.  If [Params::can_adjust_break] is false, the rendering engine
    /// assumes that all breakpoints are acceptable.
    fn adjust_break(&self, cell: &Content, size: Coord2) -> isize;

    /// Draws a generalized intersection of lines in `bb`.
    ///
    /// `styles` is interpreted this way:
    ///
    /// `styles[Axis2::X][0]`: style of line from top of `bb` to its center.
    /// `styles[Axis2::X][1]`: style of line from bottom of `bb` to its center.
    /// `styles[Axis2::Y][0]`: style of line from left of `bb` to its center.
    /// `styles[Axis2::Y][1]`: style of line from right of `bb` to its center.
    fn draw_line(&mut self, bb: Rect2, styles: EnumMap<Axis2, [BorderStyle; 2]>, bg: Color);

    /// Draws `cell` within bounding box `bb`.  `clip` is the same as `bb` (the
    /// common case) or a subregion enclosed by `bb`.  In the latter case only
    /// the part of the cell that lies within `clip` should actually be drawn,
    /// although `bb` should used to determine the layout of the cell.
    ///
    /// The text in the cell needs to be vertically offset `valign_offset` units
    /// from the top of the bounding box.  This handles vertical alignment with
    /// the cell.  (The caller doesn't just reduce the bounding box size because
    /// that would prevent the implementation from filling the entire cell with
    /// the background color.)  The implementation must handle horizontal
    /// alignment itself.
    fn draw_cell(
        &mut self,
        draw_cell: &DrawCell,
        bb: Rect2,
        valign_offset: isize,
        spill: EnumMap<Axis2, [isize; 2]>,
        clip: &Rect2,
    );

    /// Scales all output by `factor`, e.g. a `factor` of 0.5 would cause
    /// everything subsequent to be drawn half-size.  `factor` will be greater
    /// than 0 and less than or equal to 1.
    ///
    /// Optional.  If [Params::can_scale] is false, the rendering engine won't
    /// try to scale output.
    fn scale(&mut self, factor: f64);
}

#[derive(Debug)]
struct RenderedTable {
    table: Table,

    /// "Cell positions".
    ///
    /// `cp[X]` represents `x` positions within the table:
    ///
    /// - `cp[X][0]` = 0.
    /// - `cp[X][1]` = the width of the leftmost vertical rule.
    /// - `cp[X][2]` = `cp[X][1]` + the width of the leftmost column.
    /// - `cp[X][3]` = `cp[X][2]` + the width of the second-from-left vertical rule.
    /// - ...
    /// - `cp[X][2 * n[X]]` = `x` position of the rightmost vertical rule.
    /// - `cp[X][2 * n[X] + 1]` = total table width including all rules.
    ///
    /// So, for 0-based column `i`:
    ///
    /// * The rule to its left covers `cp[X][i * 2]..cp[X][i * 2 + 1]`.
    /// * The column covers `cp[X][i * 2 + 1]..cp[X][i * 2 + 2]`.
    /// * The rule to its right covers `cp[X][i * 2 + 2]..cp[X][i * 2 + 3]`.
    ///
    /// Similarly, `cp[Y]` represents `y` positions within the table:
    ///
    /// - `cp[Y][0]` = 0.
    /// - `cp[Y][1]` = the height of the topmost horizontal rule.
    /// - `cp[Y][2]` = `cp[Y][1]` + the height of the topmost row.
    /// - `cp[Y][3]` = `cp[Y][2]` + the height of the second-from-top horizontal rule.
    /// - ...
    /// - `cp[Y][2 * n[Y]]` = `y` position of the bottommost horizontal rule.
    /// - `cp[Y][2 * n[Y] + 1]` = total table height including all rules.
    ///
    /// So, for 0-based row `i`:
    ///
    /// * The rule above it covers `cp[Y][i * 2]..cp[Y][i * 2 + 1]`.
    /// * The row covers `cp[Y][i * 2 + 1]..cp[Y][i * 2 + 2]`.
    /// * The rule below it covers `cp[Y][i * 2 + 2]..cp[Y][i * 2 + 3]`.
    ///
    /// Rules and columns can have width or height 0, in which case consecutive
    /// values in this array are equal.
    cp: EnumMap<Axis2, Vec<isize>>,
}

impl RenderedTable {
    /// Creates and returns a new [RenderedTable] for rendering `table` with the given
    /// `look` on `device`.
    ///
    /// The new [Page] will be suitable for rendering on a device whose page
    /// size is `params.size`, but the caller is responsible for actually
    /// breaking it up to fit on such a device, using the [Break] abstraction.
    fn new(table: Table, device: &dyn Device, min_width: Option<isize>, look: &Look) -> Self {
        use Axis2::*;
        use Extreme::*;

        let n = table.n;

        // Figure out rule widths.
        //
        // `rules[X]` is vertical rules.
        // `rules[Y]` is horizontal rules.
        let rules = EnumMap::from_fn(|axis| {
            (0..=n[axis])
                .map(|z| measure_rule(device, &table, axis, z))
                .collect::<Vec<_>>()
        });

        let px_size = device.params().px_size.unwrap_or_default();
        let heading_widths = look.heading_widths.clone().map(|_region, range| {
            enum_map![
                Min => *range.start() * px_size,
                Max => *range.end() * px_size
            ]
        });

        // Calculate minimum and maximum widths of cells that do not span
        // multiple columns.
        let mut unspanned_columns = EnumMap::from_fn(|_| vec![0; n.x]);
        for cell in table.cells().filter(|cell| cell.col_span() == 1) {
            let mut w = device.measure_cell_width(&DrawCell::new(cell.inner(), &table));
            if device.params().px_size.is_some() {
                if let Some(region) = table.heading_region(cell.pos) {
                    let wr = &heading_widths[region];
                    if w[Min] < wr[Min] {
                        w[Min] = wr[Min];
                        if w[Min] > w[Max] {
                            w[Max] = w[Min];
                        }
                    } else if w[Max] > wr[Max] {
                        w[Max] = wr[Max];
                        if w[Max] < w[Min] {
                            w[Min] = w[Max];
                        }
                    }
                }
            }

            let x = cell.pos[X];
            for ext in [Min, Max] {
                if unspanned_columns[ext][x] < w[ext] {
                    unspanned_columns[ext][x] = w[ext];
                }
            }
        }

        // Distribute widths of spanned columns.
        let mut columns = unspanned_columns.clone();
        for cell in table.cells().filter(|cell| cell.col_span() > 1) {
            let rect = cell.rect();

            let w = device.measure_cell_width(&DrawCell::new(cell.inner(), &table));
            for ext in [Min, Max] {
                distribute_spanned_width(
                    w[ext],
                    &unspanned_columns[ext][rect.x.clone()],
                    &mut columns[ext][rect.x.clone()],
                    &rules[X][rect.x.start..rect.x.end + 1],
                );
            }
        }
        if let Some(min_width) = min_width
            && min_width > 0
        {
            for ext in [Min, Max] {
                distribute_spanned_width(
                    min_width,
                    &unspanned_columns[ext],
                    &mut columns[ext],
                    &rules[X],
                );
            }
        }

        // In pathological cases, spans can cause the minimum width of a column
        // to exceed the maximum width.  This bollixes our interpolation
        // algorithm later, so fix it up.
        for i in 0..n.x {
            if columns[Min][i] > columns[Max][i] {
                columns[Max][i] = columns[Min][i];
            }
        }

        // Decide final column widths.
        let rule_widths = rules[X].iter().copied().sum::<isize>();
        let table_widths = EnumMap::from_fn(|ext| columns[ext].iter().sum::<isize>() + rule_widths);

        let cp_x = if table_widths[Max] <= device.params().size[X] {
            // Fits even with maximum widths.  Use them.
            Self::use_row_widths(&columns[Max], &rules[X])
        } else if device.params().size[X] > table_widths[Min] {
            // Fits with minimum widths, so distribute the leftover space.
            Self::interpolate_column_widths(
                device.params().size[Axis2::X],
                &columns,
                &table_widths,
                &rules[X],
            )
        } else {
            // Doesn't fit even with minimum widths.  Assign minimums for now, and
            // later we can break it horizontally into multiple pages.
            Self::use_row_widths(&columns[Min], &rules[X])
        };

        // Calculate heights of cells that do not span multiple rows.
        let mut unspanned_rows = vec![0; n[Y]];
        for cell in table.cells().filter(|cell| cell.row_span() == 1) {
            let rect = cell.rect();

            let w = joined_width(&cp_x, rect.x.clone());
            let h = device.measure_cell_height(&DrawCell::new(cell.inner(), &table), w);

            let row = &mut unspanned_rows[cell.pos.y];
            if h > *row {
                *row = h;
            }
        }

        // Distribute heights of spanned rows.
        let mut rows = unspanned_rows.clone();
        for cell in table.cells().filter(|cell| cell.row_span() > 1) {
            let rect = cell.rect();
            let w = joined_width(&cp_x, rect.x.clone());
            let h = device.measure_cell_height(&DrawCell::new(cell.inner(), &table), w);
            distribute_spanned_width(
                h,
                &unspanned_rows[rect.y.clone()],
                &mut rows[rect.y.clone()],
                &rules[Y][rect.y.start..rect.y.end + 1],
            );
        }

        // Decide final row heights.
        let cp_y = Self::use_row_widths(&rows, &rules[Y]);

        // Measure headers.  If they are "too big", get rid of them.
        let mut h = table.h;
        for (axis, cp) in [(X, cp_x.as_slice()), (Y, cp_y.as_slice())] {
            let header_width = axis_width(cp, 0..table.h[axis]);
            let max_cell_width = (table.h[axis]..n[axis])
                .map(|z| cell_width(cp, z))
                .max()
                .unwrap_or(0);
            let threshold = device.params().size[axis];
            if header_width * 2 >= threshold || header_width + max_cell_width > threshold {
                h[axis] = 0;
            }
        }
        Self {
            table,
            cp: Axis2::new_enum(cp_x, cp_y),
        }
    }

    /// A [Page] always has the same headers as its underlying [Table].
    fn h(&self) -> CellPos {
        self.table.h
    }

    fn n(&self) -> CellPos {
        self.table.n
    }

    fn use_row_widths(rows: &[isize], rules: &[isize]) -> Vec<isize> {
        let mut vec = once(0)
            .chain(interleave(rules, rows).copied())
            .collect::<Vec<_>>();
        for i in 1..vec.len() {
            vec[i] += vec[i - 1]
        }
        vec
    }

    fn interpolate_column_widths(
        target: isize,
        columns: &EnumMap<Extreme, Vec<isize>>,
        widths: &EnumMap<Extreme, isize>,
        rules: &[isize],
    ) -> Vec<isize> {
        use Extreme::*;

        let avail = target - widths[Min];
        let wanted = widths[Max] - widths[Min];
        let mut w = wanted / 2;
        let rows_mid = zip(columns[Min].iter().copied(), columns[Max].iter().copied())
            .map(|(min, max)| {
                w += avail * (max - min);
                let extra = w / wanted;
                w -= extra * wanted;
                min + extra
            })
            .collect::<Vec<_>>();
        Self::use_row_widths(&rows_mid, rules)
    }

    /// Returns the width of `extent` along `axis`.
    fn axis_width(&self, axis: Axis2, extent: Range<usize>) -> isize {
        axis_width(&self.cp[axis], extent)
    }

    /// Returns the width of cells within `extent` along `axis`.
    fn joined_width(&self, axis: Axis2, extent: Range<usize>) -> isize {
        joined_width(&self.cp[axis], extent)
    }

    /// Returns the width of the headers along `axis`.
    ///
    /// The headers do not include the rule along the right or bottom edge of
    /// the headers; that rule is considered to be part of the top or left body
    /// cell.
    fn headers_width(&self, axis: Axis2) -> isize {
        self.axis_width(axis, rule_ofs(0)..cell_ofs(self.h()[axis]))
    }

    /// Returns the width of rule `z` along `axis`.
    fn rule_width(&self, axis: Axis2, z: usize) -> isize {
        let ofs = rule_ofs(z);
        self.axis_width(axis, ofs..ofs + 1)
    }

    /// Returns the width of rule `z` along `axis`, counting in reverse order.
    fn rule_width_r(&self, axis: Axis2, z: usize) -> isize {
        let ofs = self.rule_ofs_r(axis, z);
        self.axis_width(axis, ofs..ofs + 1)
    }

    /// Returns the offset in [Self::cp] of the rule with
    /// index `rule_index_r`, which counts from the right side (or bottom) of the page
    /// left (or up), according to `axis`, respectively.  That is,
    /// if `rule_index_r` is 0, then the offset is that of the rightmost or bottommost
    /// rule; if `rule_index_r` is 1, then the offset is that of the next rule to the left
    /// (or above); and so on.
    fn rule_ofs_r(&self, axis: Axis2, rule_index_r: usize) -> usize {
        (self.table.n[axis] - rule_index_r) * 2
    }

    /// Returns the width of cell `z` along `axis`.
    fn cell_width(&self, axis: Axis2, z: usize) -> isize {
        let ofs = cell_ofs(z);
        self.axis_width(axis, ofs..ofs + 1)
    }

    /// Returns the width of the widest cell, excluding headers, along `axis`.
    fn max_cell_width(&self, axis: Axis2) -> isize {
        (self.h()[axis]..self.n()[axis])
            .map(|z| self.cell_width(axis, z))
            .max()
            .unwrap_or(0)
    }
}

/// A layout for rendering a specific table on a specific device.
///
/// May represent the layout of an entire table presented to [Pager::new], or a
/// rectangular subregion of a table broken out using [Break::next] to allow a
/// table to be broken across multiple pages.
///
/// A page's size is not limited to the size passed in as part of [Params].
/// [Pager] breaks a [Page] into smaller [page]s that will fit in the available
/// space.
///
/// A [Page] always has the same headers as its [Table].
///
/// # Rendered cells
///
/// - The columns rendered are the leftmost `self.table.h[X]`, then `r[X]`.
/// - The rows rendered are the topmost `self.table.h[Y]`, then `r[Y]`.
#[derive(Clone, Debug)]
struct Page {
    /// Rendered table.
    table: Arc<RenderedTable>,
    ranges: EnumMap<Axis2, Range<isize>>,
}

impl Page {
    /// Creates and returns a new [RenderedTable] for rendering `table` with the given
    /// `look` on `device`.
    ///
    /// The new [Page] will be suitable for rendering on a device whose page
    /// size is `params.size`, but the caller is responsible for actually
    /// breaking it up to fit on such a device, using the [Break] abstraction.
    pub fn new(table: Table, device: &dyn Device, min_width: Option<isize>, look: &Look) -> Self {
        let table = Arc::new(RenderedTable::new(table, device, min_width, look));
        let ranges = EnumMap::from_fn(|axis| {
            table.cp[axis][1 + table.h()[axis] * 2]..table.cp[axis].last().copied().unwrap()
        });
        Self { table, ranges }
    }

    pub fn split(&self, axis: Axis2) -> Break {
        Break::new(self.clone(), axis)
    }

    fn width(&self, axis: Axis2) -> isize {
        self.table.cp[axis].last().copied().unwrap()
    }

    fn draw(&self, device: &mut dyn Device, ofs: Coord2) {
        fn overlap(a: &Range<isize>, b: &Range<isize>) -> bool {
            a.contains(&b.start) || b.contains(&a.start)
        }

        use Axis2::*;
        let cp = &self.table.cp;
        let headers = Coord2::from_fn(|a| self.table.headers_width(a));
        for (y, yr) in self.table.cp[Y]
            .iter()
            .copied()
            .tuple_windows()
            .map(|(y0, y1)| y0..y1)
            .enumerate()
            .filter_map(|(y, yr)| if y % 2 == 1 { Some((y / 2, yr)) } else { None })
        {
            if yr.start >= headers[Y] && !overlap(&yr, &self.ranges[Y]) {
                continue;
            }
            for (x, xr) in self.table.cp[X]
                .iter()
                .copied()
                .tuple_windows()
                .map(|(x0, x1)| x0..x1)
                .enumerate()
                .filter_map(|(x, xr)| if x % 2 == 1 { Some((x / 2, xr)) } else { None })
            {
                if xr.start >= headers[X] && !overlap(&xr, &self.ranges[X]) {
                    continue;
                }
                let cell = self.table.table.get(CellPos { x, y });
                // XXX skip if not top-left cell
                let rect = cell.rect();
                let mut bb = Rect2::from_fn(|a| {
                    cp[a][rect[a].start * 2 + 1]..cp[a][(rect[a].end - 1) * 2 + 2]
                });
                let mut clip = if y < self.table.h().y {
                    if x < self.table.h().x {
                        // Corner
                        bb.clone()
                    } else {
                        // Top stub
                        Rect2::new(
                            max(bb[X].start, self.ranges[X].start)
                                ..min(bb[X].end, self.ranges[X].end),
                            bb[Y].clone(),
                        )
                    }
                } else if x < self.table.h().x {
                    // Left stub
                    Rect2::new(
                        bb[X].clone(),
                        max(bb[Y].start, self.ranges[Y].start)..min(bb[Y].end, self.ranges[Y].end),
                    )
                } else {
                    // Body
                    Rect2::from_fn(|a| {
                        max(bb[a].start, self.ranges[a].start)..min(bb[a].end, self.ranges[a].end)
                    })
                };
                if clip[X].start >= clip[X].end || clip[Y].start >= clip[Y].end {
                    continue;
                }
                for a in [X, Y] {
                    if bb[a].start >= self.ranges[a].start {
                        let h = self.ranges[a].start - self.table.headers_width(a);
                        bb[a].start -= h;
                        bb[a].end -= h;
                        clip[a].start -= h;
                        clip[a].end -= h;
                    }
                }
                let draw_cell = DrawCell::new(cell.content.inner(), &self.table.table);
                let valign_offset = match draw_cell.cell_style.vert_align {
                    VertAlign::Top => 0,
                    VertAlign::Middle => self.extra_height(device, &bb, &draw_cell) / 2,
                    VertAlign::Bottom => self.extra_height(device, &bb, &draw_cell),
                };
                device.draw_cell(
                    &draw_cell,
                    bb.translate(ofs),
                    valign_offset,
                    EnumMap::from_fn(|_| [0, 0]),
                    &clip.translate(ofs),
                )
            }
        }

        for (y, yr) in self.table.cp[Y]
            .iter()
            .copied()
            .tuple_windows()
            .map(|(y0, y1)| y0..y1)
            .enumerate()
        {
            for (x, xr) in self.table.cp[X]
                .iter()
                .copied()
                .tuple_windows()
                .map(|(x0, x1)| x0..x1)
                .enumerate()
                .filter(|(x, _)| *x % 2 == 0 || y % 2 == 0)
            {
                let mut bb = Rect2::new(xr.clone(), yr.clone());

                let h = self.table.headers_width(X);
                if xr.start < h {
                } else if self.ranges[X].contains(&xr.start) {
                    bb[X].start -= self.ranges[X].start - h;
                    bb[X].end -= self.ranges[X].start - h;
                } else {
                    continue;
                }

                let h = self.table.headers_width(Y);
                if yr.start < h {
                } else if self.ranges[Y].contains(&yr.start) {
                    bb[Y].start -= self.ranges[Y].start - h;
                    bb[Y].end -= self.ranges[Y].start - h;
                } else {
                    continue;
                }

                let bg = if !self.table.table.is_empty() {
                    let x = (x / 2).min(self.table.n().x - 1);
                    let y = (y / 2).min(self.table.n().y - 1);
                    let cell = self.table.table.get(CellPos::new(x, y));
                    let area = cell.inner().area;
                    self.table.table.areas[area].font_style.bg
                } else {
                    Color::WHITE
                };

                self.draw_rule(device, ofs, CellPos { x, y }, bb, bg);
            }
        }
    }

    fn draw_rule(
        &self,
        device: &mut dyn Device,
        ofs: Coord2,
        coord: CellPos,
        bb: Rect2,
        bg: Color,
    ) {
        let styles = EnumMap::from_fn(|a: Axis2| {
            let b = !a;
            if !is_rule(coord[a]) {
                [None, None]
            } else if is_rule(coord[b]) {
                let first = if coord[b] > 0 {
                    let mut e = coord;
                    e[b] -= 1;
                    self.get_rule(a, e)
                } else {
                    None
                };

                let second = if coord[b] / 2 < self.table.n()[b] {
                    self.get_rule(a, coord)
                } else {
                    None
                };

                [first, second]
            } else {
                let rule = self.get_rule(a, coord);
                [rule, rule]
            }
        });

        if styles.values().any(|[a, b]| a.is_some() || b.is_some()) {
            const NO_BORDER: BorderStyle = BorderStyle::none();
            let styles = styles.map(|_, [a, b]| [a.unwrap_or(NO_BORDER), b.unwrap_or(NO_BORDER)]);
            device.draw_line(bb.translate(ofs), styles, bg);
        }
    }

    fn get_rule(&self, a: Axis2, coord: CellPos) -> Option<BorderStyle> {
        let coord = CellPos::from_fn(|a| coord[a] / 2);
        self.table.table.get_rule(a, coord)
    }

    fn extra_height(&self, device: &dyn Device, bb: &Rect2, cell: &DrawCell) -> isize {
        use Axis2::*;
        let height = device.measure_cell_height(cell, bb[X].len() as isize);
        bb[Y].len() as isize - height
    }
}

/// Returns the width of `extent` along `axis`.
fn axis_width(cp: &[isize], extent: Range<usize>) -> isize {
    cp[extent.end] - cp[extent.start]
}

/// Returns the width of cells within `extent` along `axis`.
fn joined_width(cp: &[isize], extent: Range<usize>) -> isize {
    axis_width(cp, cell_ofs(extent.start)..cell_ofs(extent.end) - 1)
}
/// Returns the offset in [Self::cp] of the cell with index `cell_index`.
/// That is, if `cell_index` is 0, then the offset is 1, that of the leftmost
/// or topmost cell; if `cell_index` is 1, then the offset is 3, that of the
/// next cell to the right (or below); and so on. */
fn cell_ofs(cell_index: usize) -> usize {
    cell_index * 2 + 1
}

/// Returns the offset in [Self::cp] of the rule with index `rule_index`.
/// That is, if `rule_index` is 0, then the offset is that of the leftmost
/// or topmost rule; if `rule_index` is 1, then the offset is that of the
/// next rule to the right (or below); and so on.
fn rule_ofs(rule_index: usize) -> usize {
    rule_index * 2
}

/// Returns the width of cell `z` along `axis`.
fn cell_width(cp: &[isize], z: usize) -> isize {
    let ofs = cell_ofs(z);
    axis_width(cp, ofs..ofs + 1)
}

/// Is `ofs` the offset of a rule in `cp`?
fn is_rule(z: usize) -> bool {
    z.is_even()
}

#[derive(Clone)]
pub struct RenderCell<'a> {
    rect: CellRect,
    content: &'a Content,
}

struct Selection {
    a: Axis2,
    b: Axis2,
    z0: usize,
    z1: usize,
    p0: isize,
    p1: isize,
    h: CellPos,
}

impl Selection {
    /// Returns the coordinates of `coord` as it will appear in this subpage.
    ///
    /// `coord` must be in the selected region or the results will not make
    /// sense (or will panic due to overflow).
    fn coord_to_subpage(&self, coord: CellPos) -> CellPos {
        let a = self.a;
        let b = self.b;
        let ha0 = self.h[a];
        let z = coord[a];
        let z_subpage = if (0..ha0).contains(&z) {
            z
        } else if (self.z0..self.z1).contains(&z) {
            z - self.z0 + ha0
        } else {
            unreachable!("{z} is not in {:?} or {:?}", 0..ha0, self.z0..self.z1);
        };
        CellPos::for_axis((a, z_subpage), coord[b])
    }
}

/// Maps a contiguous range of cells from a page to the underlying table along
/// the horizontal or vertical dimension.
#[derive(Copy, Clone, Debug)]
struct Map {
    /// First ordinate in the page.
    p0: usize,

    /// First ordinate in the table.
    t0: usize,

    /// `t0 - p0`.
    ofs: usize,

    /// Number of ordinates in page and table.
    n: usize,
}

/// Modifies the 'width' members of `rows` so that their sum, when added to rule
/// widths `rules[1..n - 1]`, where n is rows.len(), is at least `width`.
///
/// # Implementation
///
/// The algorithm used here is based on the following description from HTML 4:
///
/// > For cells that span multiple columns, a simple approach consists of
/// > apportioning the min/max widths evenly to each of the constituent
/// > columns.  A slightly more complex approach is to use the min/max
/// > widths of unspanned cells to weight how spanned widths are
/// > apportioned.  Experiments suggest that a blend of the two approaches
/// > gives good results for a wide range of tables.
///
/// We blend the two approaches half-and-half, except that we cannot use the
/// unspanned weights when 'total_unspanned' is 0 (because that would cause a
/// division by zero).
///
/// The calculation we want to do is this:
///
/// ```text
/// w0 = width / n
/// w1 = width * (column's unspanned width) / (total unspanned width)
/// (column's width) = (w0 + w1) / 2
/// ```
///
/// We implement it as a precise calculation in integers by multiplying `w0` and
/// `w1` by the common denominator of all three calculations (`d`), dividing
/// that out in the column width calculation, and then keeping the remainder for
/// the next iteration.
///
/// (We actually compute the unspanned width of a column as twice the unspanned
/// width, plus the width of the rule on the left, plus the width of the rule on
/// the right.  That way each rule contributes to both the cell on its left and
/// on its right.)
fn distribute_spanned_width(
    width: isize,
    unspanned: &[isize],
    spanned: &mut [isize],
    rules: &[isize],
) {
    let n = unspanned.len();
    if n == 0 {
        return;
    }

    debug_assert_eq!(spanned.len(), n);
    debug_assert_eq!(rules.len(), n + 1);

    let total_unspanned = unspanned.iter().sum::<isize>()
        + rules
            .get(1..n)
            .map_or(0, |rules| rules.iter().copied().sum::<isize>());
    if total_unspanned >= width {
        return;
    }

    let d0 = n as isize;
    let d1 = 2 * total_unspanned.max(1);
    let d = if total_unspanned > 0 {
        d0 * d1 * 2
    } else {
        d0 * d1
    };
    let mut w = d / 2;
    for x in 0..n {
        w += width * d1;
        if total_unspanned > 0 {
            let mut unspanned = unspanned[x] * 2;
            if x + 1 < n {
                unspanned += rules[x + 1];
            }
            if x > 0 {
                unspanned += rules[x];
            }
            w += width * unspanned * d0;
        }
        spanned[x] = max(spanned[x], w / d);
        w = w.checked_sub(spanned[x] * d).unwrap();
    }
}

/// Returns the width of the rule in `table` that is at offset `z` along axis
/// `a`, if rendered on `device`.
fn measure_rule(device: &dyn Device, table: &Table, a: Axis2, z: usize) -> isize {
    let b = !a;

    // Determine the types of rules that are present.
    let mut rules = EnumMap::default();
    for w in 0..table.n[b] {
        if let Some(border) = table.get_rule(a, CellPos::for_axis((a, z), w)) {
            rules[border.stroke] = true;
        }
    }

    // Turn off [Stroke::None] because it has width 0 and we needn't bother.
    // However, if the device doesn't support margins, make sure that there is
    // at least a small gap between cells (but we don't need any at the left or
    // right edge of the table).
    if rules[Stroke::None] {
        rules[Stroke::None] = false;
        if z > 0 && z < table.n[a] && !device.params().supports_margins && a == Axis2::X {
            rules[Stroke::Solid] = true;
        }
    }

    // Calculate maximum width of rules that are present.
    let line_widths = &device.params().line_widths;
    rules
        .into_iter()
        .map(
            |(rule, present)| {
                if present { line_widths[rule] } else { 0 }
            },
        )
        .max()
        .unwrap_or(0)
}

#[derive(Debug)]
pub struct Break {
    page: Page,

    /// Axis along which `page` is being broken.
    axis: Axis2,
}

impl Break {
    fn new(page: Page, axis: Axis2) -> Self {
        Self { page, axis }
    }

    fn has_next(&self) -> bool {
        !self.page.ranges[self.axis].is_empty()
    }

    /// Returns a new [Page] that is up to `size` pixels wide along the axis
    /// used for breaking.  Returns `None` if the page has already been
    /// completely broken up, or if `size` is too small to reasonably render any
    /// cells.  The latter will never happen if `size` is at least as large as
    /// the page size passed to [Page::new] along the axis using for breaking.
    fn next(&mut self, device: &dyn Device, size: isize) -> Result<Option<Page>, ()> {
        if !self.has_next() {
            return Ok(None);
        }
        let target = size - self.page.table.headers_width(self.axis);
        if target <= 0 {
            return Err(());
        }
        let start = self.page.ranges[self.axis].start;
        let (end, next_start) = self.find_breakpoint(start..start + target, device);
        let result = Page {
            table: self.page.table.clone(),
            ranges: EnumMap::from_fn(|axis| {
                if axis == self.axis {
                    start..end
                } else {
                    self.page.ranges[axis].clone()
                }
            }),
        };
        self.page.ranges[self.axis].start = next_start;
        Ok(Some(result))
    }

    fn find_breakpoint(&self, range: Range<isize>, device: &dyn Device) -> (isize, isize) {
        let cp = &self.page.table.cp[self.axis];

        // If everything remaining fits, then take it all.
        let max = cp.last().copied().unwrap();
        if range.end >= max {
            return (max, max);
        }

        // Otherwise, take as much as fits.
        for c in 0..self.page.table.n()[self.axis] {
            let position = cp[c * 2 + 3];
            if position > range.end {
                if c == 0
                    || self.page.table.cell_width(self.axis, c)
                        >= device.params().min_break[self.axis]
                {
                    // XXX various way to choose a better breakpoint
                    return (range.end, range.end);
                } else {
                    return (cp[(c - 1) * 2 + 3], cp[(c - 1) * 2 + 2]);
                }
            }
        }
        unreachable!()
    }
}

pub struct Pager {
    scale: f64,

    /// [Page]s to be rendered, in order, vertically.  There may be up to 5
    /// pages, for the pivot table's title, layers, body, captions, and
    /// footnotes.
    pages: SmallVec<[Page; 5]>,

    x_break: Option<Break>,
    y_break: Option<Break>,
}

impl Pager {
    pub fn new(
        device: &dyn Device,
        pivot_table: &PivotTable,
        layer_indexes: Option<&[usize]>,
    ) -> Self {
        let output = pivot_table.output(
            layer_indexes.unwrap_or(pivot_table.layer()),
            device.params().printing,
        );

        // Figure out the width of the body of the table. Use this to determine
        // the base scale.
        let body_page = Page::new(output.body, device, None, &pivot_table.style.look);
        let body_width = body_page.width(Axis2::X).min(device.params().size.x());
        let mut scale = if body_width > device.params().size[Axis2::X]
            && pivot_table.style.look.shrink_to_fit[Axis2::X]
            && device.params().can_scale
        {
            device.params().size[Axis2::X] as f64 / body_width as f64
        } else {
            1.0
        };

        let mut pages = SmallVec::new();
        if let Some(title) = output.title {
            pages.push(Page::new(
                title,
                device,
                Some(body_width),
                &pivot_table.style.look,
            ));
        }
        for layer in output.layers {
            pages.push(Page::new(layer, device, None, &pivot_table.style.look));
        }
        pages.push(body_page);
        for table in [output.caption, output.footnotes].into_iter().flatten() {
            pages.push(Page::new(table, device, None, &pivot_table.style.look));
        }
        pages.reverse();

        // If we're shrinking tables to fit the page length, then adjust the
        // scale factor.
        //
        // XXX This will sometimes shrink more than needed, because adjusting
        // the scale factor allows for cells to be "wider", which means that
        // sometimes they won't break across as much vertical space, thus
        // shrinking the table vertically more than the scale would imply.
        // Shrinking only as much as necessary would require an iterative
        // search.
        if pivot_table.style.look.shrink_to_fit[Axis2::Y] && device.params().can_scale {
            let total_height = pages
                .iter()
                .map(|page: &Page| page.width(Axis2::Y))
                .sum::<isize>() as f64;
            let max_height = device.params().size[Axis2::Y] as f64;
            if total_height * scale >= max_height {
                scale *= max_height / total_height;
            }
        }

        Self {
            scale,
            pages,
            x_break: None,
            y_break: None,
        }
    }

    /// True if there's content left to render.
    pub fn has_next(&mut self, device: &dyn Device) -> Option<&mut Break> {
        // If there's a nonempty y_break, return it.
        if let Some(y_break) = self.y_break.as_mut()
            && y_break.has_next()
        {
            return self.y_break.as_mut();
        }

        loop {
            // Get a new y_break from the x_break.
            if let Some(x_break) = &mut self.x_break
                && let Some(page) = x_break
                    .next(
                        device,
                        (device.params().size[Axis2::X] as f64 / self.scale) as isize,
                    )
                    .unwrap()
            {
                self.y_break = Some(page.split(Axis2::Y));
                return self.y_break.as_mut();
            }

            self.x_break = Some(self.pages.pop()?.split(Axis2::X));
        }
    }

    /// Draws a chunk of content to fit in a space that has vertical size
    /// `space` and the horizontal size specified in the device parameters.
    /// Returns the amount of vertical space actually used by the rendered
    /// chunk, which will be 0 if `space` is too small to render anything or if
    /// no content remains (use [Self::has_next] to distinguish these cases).
    pub fn draw_next(&mut self, device: &mut dyn Device, mut space: isize) -> isize {
        use Axis2::*;

        if self.scale != 1.0 {
            device.scale(self.scale);
            space = (space as f64 / self.scale) as isize;
        }

        let mut ofs = Coord2::new(0, 0);
        while let Some(y_break) = self.has_next(device) {
            let Some(page) = y_break.next(device, space - ofs[Y]).unwrap_or_default() else {
                break;
            };
            page.draw(device, ofs);
            ofs[Y] += page.width(Y);
        }

        if self.scale != 1.0 {
            ofs[Y] = (ofs[Y] as f64 * self.scale) as isize;
        }
        ofs[Y]
    }
}
