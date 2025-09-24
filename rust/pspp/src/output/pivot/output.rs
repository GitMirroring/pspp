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
    iter::{once, repeat},
    ops::Range,
    sync::Arc,
};

use enum_map::{EnumMap, enum_map};
use itertools::Itertools;

use crate::output::{
    pivot::{
        Footnote, Path,
        look::{HeadingRegion, LabelPosition, RowParity},
    },
    table::{CellInner, CellPos, CellRect, Table},
};

use crate::output::pivot::{
    Axis2, Axis3, Dimension, PivotTable,
    look::{Area, Border, BorderStyle, BoxBorder, Color, RowColBorder, Stroke},
    value::Value,
};

/// All of the combinations of dimensions along an axis.
struct AxisEnumeration {
    indexes: Vec<usize>,
    stride: usize,
}

impl AxisEnumeration {
    fn len(&self) -> usize {
        self.indexes.len() / self.stride
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, index: usize) -> &[usize] {
        let start = self.stride * index;
        &self.indexes[start..start + self.stride]
    }

    fn iter(&self) -> AxisEnumerationIter<'_> {
        AxisEnumerationIter {
            enumeration: self,
            position: 0,
        }
    }
}

struct AxisEnumerationIter<'a> {
    enumeration: &'a AxisEnumeration,
    position: usize,
}

impl<'a> Iterator for AxisEnumerationIter<'a> {
    type Item = &'a [usize];

    fn next(&mut self) -> Option<Self::Item> {
        if self.position < self.enumeration.indexes.len() {
            let item =
                &self.enumeration.indexes[self.position..self.position + self.enumeration.stride];
            self.position += self.enumeration.stride;
            Some(item)
        } else {
            None
        }
    }
}

impl PivotTable {
    fn is_row_empty(
        &self,
        layer_indexes: &[usize],
        fixed_indexes: &[usize],
        fixed_axis: Axis3,
    ) -> bool {
        let vary_axis = fixed_axis.transpose().unwrap();
        for vary_indexes in self.axis_values(vary_axis) {
            let mut presentation_indexes = enum_map! {
                Axis3::Z => layer_indexes,
                _ => fixed_indexes,
            };
            presentation_indexes[vary_axis] = &vary_indexes;
            let data_indexes = self.convert_indexes_ptod(presentation_indexes);
            if self.get(&*data_indexes).is_some() {
                return false;
            }
        }
        true
    }
    fn enumerate_axis(
        &self,
        enum_axis: Axis3,
        layer_indexes: &[usize],
        omit_empty: bool,
    ) -> AxisEnumeration {
        let axis = &self.structure.axes[enum_axis];
        let extent = self.axis_extent(enum_axis);
        let indexes = if axis.dimensions.is_empty() {
            vec![0]
        } else if extent == 0 {
            vec![]
        } else {
            let mut enumeration =
                Vec::with_capacity(extent.checked_mul(axis.dimensions.len()).unwrap());
            if omit_empty {
                for axis_indexes in self.axis_values(enum_axis) {
                    if !self.is_row_empty(layer_indexes, &axis_indexes, enum_axis) {
                        enumeration.extend_from_slice(&axis_indexes);
                    }
                }
            }

            if enumeration.is_empty() {
                for axis_indexes in self.axis_values(enum_axis) {
                    enumeration.extend_from_slice(&axis_indexes);
                }
            }
            enumeration
        };
        AxisEnumeration {
            indexes,
            stride: axis.dimensions.len().max(1),
        }
    }

    fn create_aux_table<I>(&self, area: Area, axis: Axis2, cells: I) -> Table
    where
        I: IntoIterator<Item = Box<Value>>,
        I::IntoIter: ExactSizeIterator,
    {
        let cells = cells.into_iter();
        let mut table = Table::new(
            CellPos::for_axis((axis, cells.len()), 1),
            CellPos::new(0, 0),
            self.style.look.areas.clone(),
            self.borders(false),
            self,
        );
        for (z, row) in cells.into_iter().enumerate() {
            table.put(
                CellRect::for_cell(CellPos::for_axis((axis, z), 0)),
                CellInner::new(area, row),
            );
        }
        table
    }

    fn create_aux_table_if_nonempty<I>(&self, area: Area, rows: I) -> Option<Table>
    where
        I: IntoIterator<Item = Box<Value>>,
        I::IntoIter: ExactSizeIterator,
    {
        let rows = rows.into_iter();
        if rows.len() > 0 {
            Some(self.create_aux_table(area, Axis2::Y, rows))
        } else {
            None
        }
    }

    fn borders(&self, printing: bool) -> EnumMap<Border, BorderStyle> {
        fn resolve_border_style(
            border: Border,
            borders: &EnumMap<Border, BorderStyle>,
            show_grid_lines: bool,
        ) -> BorderStyle {
            // Use the style for `border` if it's non-`None`.
            let style = borders[border];
            if style.stroke != Stroke::None {
                return style;
            }

            // Use the fallback style, if any, if it's non-`None`.
            if let Some(fallback) = border.fallback()
                && let style = borders[fallback]
                && style.stroke != Stroke::None
            {
                return style;
            }

            // Show grid lines.
            if show_grid_lines {
                return BorderStyle {
                    stroke: Stroke::Dashed,
                    color: Color::BLACK,
                };
            }

            // Use `None` after all.
            style
        }

        EnumMap::from_fn(|border| {
            resolve_border_style(
                border,
                &self.style.look.borders,
                printing && self.style.show_grid_lines,
            )
        })
    }

    /// Constructs a [Table] for the body of this `PivotTable` for the layer
    /// with the specified indexes.  `printing` specifies whether the table is
    /// for printing or screen display (grid lines are only enabled for
    /// printing).
    pub fn output_body(&self, layer_indexes: &[usize], printing: bool) -> Table {
        let headings = EnumMap::from_fn(|axis| Headings::new(self, axis, layer_indexes));

        let data = CellPos::from_fn(|axis| headings[axis].width());
        let mut stub = CellPos::from_fn(|axis| headings[!axis].height());
        if headings[Axis2::Y].row_label_position == LabelPosition::Corner && stub.y == 0 {
            stub[Axis2::Y] = 1;
        }
        let mut body = Table::new(
            CellPos::from_fn(|axis| data[axis] + stub[axis]),
            stub,
            self.style.look.areas.clone(),
            self.borders(printing),
            self,
        );

        for h in [Axis2::X, Axis2::Y] {
            headings[h].render(&mut body, stub[h], h.into(), false, false);
        }

        for (row_indexes, y) in headings[Axis2::Y].values.iter().zip(stub[Axis2::Y]..) {
            for (column_indexes, x) in headings[Axis2::X].values.iter().zip(stub[Axis2::X]..) {
                let presentation_indexes = enum_map! {
                    Axis3::X => &column_indexes,
                    Axis3::Y => &row_indexes,
                    Axis3::Z => layer_indexes,
                };
                let data_indexes = self.convert_indexes_ptod(presentation_indexes);
                let value = self.get(&*data_indexes);
                body.put(
                    CellRect::new(x..x + 1, y..y + 1),
                    CellInner::new(
                        Area::Data(RowParity::from(y - stub[Axis2::Y])),
                        Box::new(value.cloned().unwrap_or_default()),
                    ),
                );
            }
        }

        // Insert corner text, but only if there's a stub and only if row labels
        // are not in the corner.
        if self.metadata.corner_text.is_some()
            && self.style.look.row_label_position == LabelPosition::Nested
            && stub.x > 0
            && stub.y > 0
        {
            body.put(
                CellRect::new(0..stub.x, 0..stub.y),
                CellInner::new(
                    Area::Corner,
                    self.metadata.corner_text.clone().unwrap_or_default(),
                ),
            );
        }

        if body.n.x > 0 && body.n.y > 0 {
            body.h_line(Border::InnerFrame(BoxBorder::Top), 0..body.n.x, 0);
            body.h_line(Border::InnerFrame(BoxBorder::Bottom), 0..body.n.x, body.n.y);
            body.v_line(Border::InnerFrame(BoxBorder::Left), 0, 0..body.n.y);
            body.v_line(Border::InnerFrame(BoxBorder::Right), body.n.x, 0..body.n.y);

            body.h_line(Border::DataTop, 0..body.n.x, stub.y);
            body.v_line(Border::DataLeft, stub.x, 0..body.n.y);
        }
        body
    }

    /// Constructs a [Table] for this `PivotTable`'s title.  Returns `None` if
    /// the table doesn't have a title.
    pub fn output_title(&self) -> Option<Table> {
        Some(self.create_aux_table(
            Area::Title,
            Axis2::Y,
            [self.metadata.title.as_ref()?.clone()],
        ))
    }

    /// Constructs a [Table] for this `PivotTable`'s layer values.  Returns
    /// `None` if the table doesn't have layers.
    pub fn output_layers(&self, layer_indexes: &[usize]) -> Vec<Table> {
        self.structure.axes[Axis3::Z]
            .dimensions
            .iter()
            .map(|index| &self.structure.dimensions[*index])
            .zip(layer_indexes)
            .rev()
            .filter(|(dimension, _)| !dimension.is_empty() && !dimension.hide_all_labels)
            .map(|(dimension, &layer_index)| {
                let mut cells = Vec::with_capacity(4);
                let (groups, leaf) = dimension.leaf_path(layer_index).unwrap().into_parts();
                for (group, separator) in groups
                    .iter()
                    .zip(once(": ").chain(repeat(" ")))
                    .filter(|(group, _separator)| group.show_label)
                {
                    cells.push(Box::new(group.name().clone()));
                    cells.push(Box::new(Value::new_user_text(separator)));
                }
                cells.push(Box::new(leaf.name().clone()));
                self.create_aux_table(Area::Layers, Axis2::X, cells)
            })
            .collect()
    }

    /// Constructs a [Table] for this `PivotTable`'s caption.  Returns `None` if
    /// the table doesn't have a caption.
    pub fn output_caption(&self) -> Option<Table> {
        Some(self.create_aux_table(
            Area::Caption,
            Axis2::Y,
            [self.metadata.caption.as_ref()?.clone()],
        ))
    }

    /// Constructs a [Table] for this `PivotTable`'s footnotes.  Returns `None`
    /// if the table doesn't have footnotes.
    pub fn output_footnotes(&self, footnotes: &[Arc<Footnote>]) -> Option<Table> {
        self.create_aux_table_if_nonempty(
            Area::Footer,
            footnotes.iter().map(|f| {
                Box::new(Value::new_user_text(format!(
                    "{}. {}",
                    f.display_marker(self),
                    f.display_content(self)
                )))
            }),
        )
    }

    /// Constructs [OutputTables] for this `PivotTable`, for the specified
    /// layer, formatted for screen display or printing as specified.
    pub fn output(&self, layer_indexes: &[usize], printing: bool) -> OutputTables {
        // Produce most of the tables.
        let title = self.style.show_title.then(|| self.output_title()).flatten();
        let layers = self.output_layers(layer_indexes);
        let body = self.output_body(layer_indexes, printing);
        let caption = self
            .style
            .show_caption
            .then(|| self.output_caption())
            .flatten();

        // Then collect the footnotes from those tables.
        let title_iter = once(title.as_ref()).flatten();
        let layers_iter = layers.iter();
        let body_iter = once(&body);
        let caption_iter = once(caption.as_ref()).flatten();
        let tables_iter = title_iter
            .chain(layers_iter)
            .chain(body_iter)
            .chain(caption_iter);
        let footnotes = self.output_footnotes(&self.collect_footnotes(tables_iter));

        OutputTables {
            title,
            layers,
            body,
            caption,
            footnotes,
        }
    }

    fn nonempty_layer_dimensions(&self) -> impl Iterator<Item = &Dimension> {
        self.structure.axes[Axis3::Z]
            .dimensions
            .iter()
            .rev()
            .map(|index| &self.structure.dimensions[*index])
            .filter(|d| !d.root.is_empty())
    }

    fn collect_footnotes<'a>(&self, tables: impl Iterator<Item = &'a Table>) -> Vec<Arc<Footnote>> {
        if self.footnotes.is_empty() {
            return Vec::new();
        }

        let mut refs = Vec::with_capacity(self.footnotes.0.len());
        for table in tables {
            for cell in table.cells() {
                if let Some(styling) = &cell.inner().value.styling {
                    refs.extend(
                        styling
                            .footnotes
                            .iter()
                            .filter(|footnote| footnote.show)
                            .cloned(),
                    );
                }
            }
        }
        refs.sort_by_key(|f| f.index);
        refs.dedup_by_key(|f| f.index);
        refs
    }
}

/// [Table]s for outputting a layer of a [PivotTable].
pub struct OutputTables {
    /// Title table, if any.
    pub title: Option<Table>,
    /// Layers tables, if any.
    pub layers: Vec<Table>,
    /// Table body.
    pub body: Table,
    /// Table caption, if any.
    pub caption: Option<Table>,
    /// Footnotes, if any.
    pub footnotes: Option<Table>,
}

impl Path<'_> {
    /// Gets the label to be displayed for this path to a leaf within a heading
    /// block with the given `height`.  Returns both the label and the range of
    /// rows within the heading block that displays the label.
    ///
    /// A path to a leaf that contains `n` groups must be displayed in a heading
    /// block with at least `n + 1` rows.  Within a heading block with `height`
    /// rows, the groups are displayed in rows `0..n`, and the leaf is displayed
    /// in rows `n..height`.  Thus, each group is displayed in exactly one row,
    /// but the leaf can span multiple rows.
    pub fn get(&self, y: usize, height: usize) -> (&Value, Range<usize>) {
        debug_assert!(height > self.groups.len());
        if let Some(group) = self.groups.get(y) {
            (&*group.name, y..y + 1)
        } else {
            (&self.leaf.name, self.groups.len()..height)
        }
    }
}

struct Heading<'a> {
    dimension: &'a Dimension,
    height: usize,
    columns: Vec<Path<'a>>,
}

impl<'a> Heading<'a> {
    fn new(
        dimension: &'a Dimension,
        dim_index: usize,
        column_enumeration: &AxisEnumeration,
    ) -> Option<Self> {
        if dimension.hide_all_labels {
            return None;
        }

        let mut columns = Vec::new();
        let mut height = 0;
        for indexes in column_enumeration.iter() {
            let mut path = dimension.leaf_path(indexes[dim_index]).unwrap();
            path.groups.retain(|group| group.show_label);
            height = height.max(1 + path.groups.len());
            columns.push(path);
        }

        Some(Self {
            dimension,
            height,
            columns,
        })
    }

    fn move_dimension_labels_to_corner(&mut self) -> bool {
        if self.dimension.root.show_label {
            for column in self.columns.iter_mut() {
                column.groups.remove(0);
            }
            self.height -= 1;
            true
        } else {
            false
        }
    }

    fn render(
        &self,
        table: &mut Table,
        vrules: &mut [bool],
        h: Axis2,
        h_ofs: usize,
        v_ofs: usize,
        region: HeadingRegion,
        rotate_inner_labels: bool,
        rotate_outer_labels: bool,
        inner: bool,
        n_dimensions: usize,
    ) {
        let v = !h;

        // Go through the heading row by row.
        for row in 0..self.height {
            // Find all the categories, dropping columns without a category.
            let categories = self.columns.iter().enumerate().filter_map(|(x, column)| {
                let (name, y_range) = column.get(row, self.height);
                (y_range.start == row).then_some((x..x + 1, y_range, name))
            });

            // Merge adjacent identical categories (but don't merge across a vertical rule).
            let categories = categories
                .coalesce(|(a_r, a_yr, a), (b_r, b_yr, b)| {
                    if a_r.end == b_r.start && !vrules[b_r.start] && std::ptr::eq(a, b) {
                        Ok((a_r.start..b_r.end, a_yr, a))
                    } else {
                        Err(((a_r, a_yr, a), (b_r, b_yr, b)))
                    }
                })
                .collect::<Vec<_>>();
            for (Range { start: x1, end: x2 }, yr, name) in categories.iter().cloned() {
                let y1 = v_ofs + yr.start;
                let y2 = v_ofs + yr.end;

                let is_outer_row = y1 == 0;
                let is_inner_row = y2 == self.height;
                let rotate =
                    (rotate_inner_labels && is_inner_row) || (rotate_outer_labels && is_outer_row);
                table.put(
                    CellRect::for_ranges((h, x1 + h_ofs..x2 + h_ofs), y1..y2),
                    CellInner::new(Area::Labels(h), Box::new(name.clone())).with_rotate(rotate),
                );

                // Draw all the vertical lines in our running example, other
                // than the far left and far right ones.
                //
                // On an axis with only one dimension, all the lines are drawn
                // in "category" style.
                //
                // On an axis with multiple dimensions, lines that start at the
                // innermost leaf categories are drawn in "category" style, and
                // all the other lines (which start above the leaves) are drawn
                // in "dimension" style.
                //
                // # Example
                //
                // Suppose we have four dimensions `a` through `d`, each with
                // three numbered categories `a1`, `a2`, `a3` (etc.).  Two of
                // the categories in each dimension are grouped into `ag1`
                // (etc.).  Then, only the doubled lines below are category
                // style:
                //
                // ```text
                // Category and Dimension Borders 1
                //                            b            │
                //                      bg1       │        │
                //                  b1   │   b2   │   b3   │
                //                   a   │    a   │    a   │
                //                 │ ag1 │  │ ag1 │  │ ag1 │
                // d      c      a1│a2║a3│a1│a2║a3│a1│a2║a3│
                // dg1 d1 c1      0│ 1║ 2│ 3│ 4║ 5│ 6│ 7║ 8│
                //       ╶─────────┼──╫──┼──┼──╫──┼──┼──╫──┤
                //        cg1 c2  9│10║11│12│13║14│15│16║17│
                //           ══════╪══╬══╪══╪══╬══╪══╪══╬══╡
                //            c3 18│19║20│21│22║23│24│25║26│
                //    ╶────────────┼──╫──┼──┼──╫──┼──┼──╫──┤
                //     d2 c1     27│28║29│30│31║32│33│34║35│
                //       ╶─────────┼──╫──┼──┼──╫──┼──┼──╫──┤
                //        cg1 c2 36│37║38│39│40║41│42│43║44│
                //           ══════╪══╬══╪══╪══╬══╪══╪══╬══╡
                //            c3 45│46║47│48│49║50│51│52║53│
                // ────────────────┼──╫──┼──┼──╫──┼──┼──╫──┤
                // d3     c1     54│55║56│57│58║59│60│61║62│
                //       ╶─────────┼──╫──┼──┼──╫──┼──┼──╫──┤
                //        cg1 c2 63│64║65│66│67║68│69│70║71│
                //           ══════╪══╬══╪══╪══╬══╪══╪══╬══╡
                //            c3 72│73║74│75│76║77│78│79║80│
                // ────────────────┴──╨──┴──┴──╨──┴──┴──╨──╯
                // ```
                //
                // (This is [tests::category_and_dimension_borders_1] with
                // double instead of dashed lines, because double lines are
                // easier to see in source code but SPSS shows rendering
                // anomalies with them.)
                let row_col = RowColBorder(region, v);
                let border = if n_dimensions > 1 && (!inner || row != self.height - 1) {
                    Border::Dimension(row_col)
                } else {
                    Border::Category(row_col)
                };
                for x in [x1, x2] {
                    if !vrules[x] {
                        table.draw_line(border, (v, x + h_ofs), y1..table.n[v]);
                        vrules[x] = true;
                    }
                }

                // Draw the horizontal lines within a dimension, that is, those
                // that separate a category (or group) from its parent group or
                // dimension's label.
                //
                // # Example
                //
                // Our running example doesn't have groups but the `═════` lines
                // below show the separators between categories and their
                // dimension label:
                //
                // ```text
                // ┌─────────────────────────────────────────────────────┐
                // │                         bbbb                        │
                // ╞═════════════════╤═════════════════╤═════════════════╡
                // │      bbbb1      │      bbbb2      │      bbbb3      │
                // ├─────────────────┼─────────────────┼─────────────────┤
                // │       aaaa      │       aaaa      │       aaaa      │
                // ╞═════╤═════╤═════╪═════╤═════╤═════╪═════╤═════╤═════╡
                // │aaaa1│aaaa2│aaaa3│aaaa1│aaaa2│aaaa3│aaaa1│aaaa2│aaaa3│
                // └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘
                // ```
                if row + 1 < self.height {
                    table.draw_line(Border::Category(RowColBorder(region, h)), (h, y2), {
                        if row == 0
                            && !self.columns[0].groups.is_empty()
                            && std::ptr::eq(
                                &*self.columns[0].groups[0].name,
                                &*self.dimension.root.name,
                            )
                        {
                            h_ofs..table.n[h]
                        } else {
                            h_ofs + x1..h_ofs + x2
                        }
                    });
                }
            }
        }
    }
}

struct Headings<'a> {
    headings: Vec<Heading<'a>>,
    row_label_position: LabelPosition,
    h: Axis2,
    values: AxisEnumeration,
}

impl<'a> Headings<'a> {
    fn new(pt: &'a PivotTable, h: Axis2, layer_indexes: &[usize]) -> Self {
        let column_enumeration =
            pt.enumerate_axis(h.into(), layer_indexes, pt.style.look.hide_empty);

        let mut headings = pt.structure.axes[h.into()]
            .dimensions
            .iter()
            .copied()
            .enumerate()
            .rev()
            .filter_map(|(axis_index, dim_index)| {
                Heading::new(
                    &pt.structure.dimensions[dim_index],
                    axis_index,
                    &column_enumeration,
                )
            })
            .collect::<Vec<_>>();

        let row_label_position = if h == Axis2::Y
            && pt.style.look.row_label_position == LabelPosition::Corner
            && headings
                .iter_mut()
                .map(|heading| heading.move_dimension_labels_to_corner())
                .filter(|x| *x)
                .count()
                > 0
        {
            LabelPosition::Corner
        } else {
            LabelPosition::Nested
        };

        Self {
            headings,
            row_label_position,
            h,
            values: column_enumeration,
        }
    }

    fn height(&self) -> usize {
        self.headings.iter().map(|h| h.height).sum()
    }

    fn width(&self) -> usize {
        self.values.len()
    }

    fn render(
        &self,
        table: &mut Table,
        h_ofs: usize,
        region: HeadingRegion,
        rotate_inner_labels: bool,
        rotate_outer_labels: bool,
    ) {
        if self.headings.is_empty() {
            return;
        }

        let h = self.h;
        let n_columns = self.width();
        let mut vrules = vec![false; n_columns + 1];
        vrules[0] = true;
        vrules[n_columns] = true;

        let mut v_ofs = 0;
        for (index, heading) in self.headings.iter().enumerate() {
            let inner = index == self.headings.len() - 1;
            heading.render(
                table,
                &mut vrules,
                h,
                h_ofs,
                v_ofs,
                region,
                rotate_inner_labels,
                rotate_outer_labels,
                inner,
                self.headings.len(),
            );
            v_ofs += heading.height;
            if !inner {
                // Draw the horizontal line between dimensions.
                //
                // # Example
                //
                // Suppose we have two dimensions `aaaa` and `bbbb`, each with
                // three numbered categories.  This code draws the `=====` line
                // here:
                //
                // ```text
                // ┌─────────────────────────────────────────────────────┐ __
                // │                         bbbb                        │  │
                // ├─────────────────┬─────────────────┬─────────────────┤  │dim "bbbb"
                // │      bbbb1      │      bbbb2      │      bbbb3      │ _│
                // ╞═════════════════╪═════════════════╪═════════════════╡ __
                // │       aaaa      │       aaaa      │       aaaa      │  │
                // ├─────┬─────┬─────┼─────┬─────┬─────┼─────┬─────┬─────┤  │dim "aaaa"
                // │aaaa1│aaaa2│aaaa3│aaaa1│aaaa2│aaaa3│aaaa1│aaaa2│aaaa3│ _│
                // └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘
                // ```
                table.draw_line(
                    Border::Dimension(RowColBorder(region, h)),
                    (h, v_ofs),
                    h_ofs..table.n[h],
                );
            }
        }

        // Display dimension labels in the corner.
        //
        // We allow a corner dimension label to spill over into additional
        // otherwise blank columns in the stub, which can save horizontal space.
        // For example, it can change this table:
        //
        // ```text
        // Data File and Variable Attributes
        // ╭────────────────────────┬─────╮
        // │Variable and Name       │Value│
        // ├────────────────────────┼─────┤
        // │variable0         $@Role│0    │
        // ├────────────────────────┼─────┤
        // │variable1         $@Role│0    │
        // ├────────────────────────┼─────┤
        // │variable2         $@Role│0    │
        // ├────────────────────────┼─────┤
        // │variable3         $@Role│0    │
        // ╰────────────────────────┴─────╯
        // ```
        //
        // into this one:
        //
        // ```text
        // Data File and Variable Attributes
        // ╭──────────────────┬─────╮
        // │Variable and Name │Value│
        // ├──────────────────┼─────┤
        // │variable0 $@Role  │0    │
        // ├──────────────────┼─────┤
        // │variable1 $@Role  │0    │
        // ├──────────────────┼─────┤
        // │variable2 $@Role  │0    │
        // ├──────────────────┼─────┤
        // │variable3 $@Role  │0    │
        // ╰──────────────────┴─────╯
        // ```
        if self.row_label_position == LabelPosition::Corner {
            let mut corner_labels = Vec::new();
            let mut x = 0;
            for heading in &self.headings {
                if heading.dimension.root.show_label {
                    corner_labels.push((x, heading));
                }
                x += heading.height;
            }
            for (i, (x0, heading)) in corner_labels.iter().copied().enumerate() {
                let x1 = corner_labels
                    .get(i + 1)
                    .map_or(table.h[Axis2::X], |(x, _heading)| *x);
                table.put(
                    CellRect::new(x0..x1, 0..table.h[Axis2::Y]),
                    CellInner::new(Area::Corner, heading.dimension.root.name.clone()),
                );
            }
        }
    }
}
