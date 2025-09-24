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

//! Pivot tables.
//!
//! Pivot tables are PSPP's primary form of output.  They are analogous to the
//! pivot tables you might be familiar with from spreadsheets and databases.
//! See <https://en.wikipedia.org/wiki/Pivot_table> for a brief introduction to
//! the overall concept of a pivot table.
//!
//! In PSPP, the most important internal pieces of a pivot table are:
//!
//! - Title.  Every pivot table has a title that is displayed above it.  It also
//!   has an optional caption (displayed below it) and corner text (displayed in
//!   the upper left corner).
//!
//! - Dimensions.  A dimension consists of zero or more categories.  A category
//!   has a label, such as "df" or "Asymp. Sig." or 123 or a variable name.  The
//!   categories are the leaves of a tree whose non-leaf nodes form groups of
//!   categories.  The tree always has a root group whose label is the name of
//!   the dimension.
//!
//! - Axes.  A table has three axes: column, row, and layer.  Each dimension is
//!   assigned to an axis, and each axis has zero or more dimensions.  When an
//!   axis has more than one dimension, they are ordered from innermost to
//!   outermost.
//!
//! - Data.  A table's data consists of zero or more cells.  Each cell maps from
//!   a category for each dimension to a value, which is commonly a number but
//!   could also be a variable name or an arbitrary text string.

// Warn about missing docs, but not for items declared with `#[cfg(test)]`.
#![cfg_attr(not(test), warn(missing_docs))]

use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    iter::{FusedIterator, once, repeat_n, zip},
    ops::{Index, IndexMut, Not, Range},
    sync::Arc,
};

use chrono::{NaiveDateTime, Utc};
use enum_iterator::Sequence;
use enum_map::{Enum, EnumMap, enum_map};
use itertools::Itertools;
pub use look_xml::{Length, TableProperties};
use serde::{Deserialize, Serialize, ser::SerializeMap};
use smallvec::{SmallVec, smallvec};

use crate::{
    format::{Decimal, F40, F40_2, F40_3, Format, PCT40_1, Settings as FormatSettings},
    output::pivot::{
        look::{Look, Sizing},
        value::{BareValue, Value, ValueFormat, ValueOptions},
    },
    settings::{Settings, Show},
    variable::Variable,
};
pub(crate) use tlo::parse_bool;

mod output;
pub use output::OutputTables;
mod look_xml;
mod tlo;
pub mod value;

#[cfg(test)]
pub mod tests;

pub mod look;

/// A 3-dimensional axis.
#[derive(Copy, Clone, Debug, Enum, PartialEq, Eq, Sequence, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis3 {
    /// X axis.
    X,
    /// Y axis.
    Y,
    /// Z axis.
    Z,
}

impl Axis3 {
    /// Transposes the X and Y axes.  Returns `None` if this represents the Z
    /// axis.
    pub fn transpose(&self) -> Option<Self> {
        match self {
            Axis3::X => Some(Axis3::Y),
            Axis3::Y => Some(Axis3::X),
            Axis3::Z => None,
        }
    }
}

impl From<Axis2> for Axis3 {
    fn from(axis2: Axis2) -> Self {
        match axis2 {
            Axis2::X => Self::X,
            Axis2::Y => Self::Y,
        }
    }
}

/// An axis within a pivot table.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Axis {
    /// `dimensions[0]` is the innermost dimension.
    pub dimensions: Vec<usize>,
}

/// Iterator over one of the [Axis3] axes in a [PivotTable].
///
/// The items for this iterator are the index values for each of the dimensions
/// along the axis, each along `0..n` where `n` is the number of leaves in the
/// dimension.
///
/// Use [PivotTable::axis_values] to construct an `AxisIterator`.
struct AxisIterator {
    indexes: SmallVec<[usize; 4]>,
    lengths: SmallVec<[usize; 4]>,
    done: bool,
}

impl FusedIterator for AxisIterator {}
impl Iterator for AxisIterator {
    type Item = SmallVec<[usize; 4]>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            None
        } else {
            let retval = self.indexes.clone();
            for (index, len) in self.indexes.iter_mut().zip(self.lengths.iter().copied()) {
                *index += 1;
                if *index < len {
                    return Some(retval);
                };
                *index = 0;
            }
            self.done = true;
            Some(retval)
        }
    }
}

impl PivotTable {
    /// Constructs a new `PivotTable` with the given `dimensions` along the
    /// specified axes.
    ///
    /// The caller should add a title to the pivot table using [with_title] and
    /// add data with [with_data] or [insert].
    ///
    /// [with_title]: Self::with_title
    /// [with_data]: Self::with_data
    /// [insert]: Self::insert
    pub fn new(structure: impl Into<Structure>) -> Self {
        let structure = structure.into();
        Self {
            style: PivotTableStyle::default().with_look(Settings::global().look.clone()),
            layer: repeat_n(0, structure.axes[Axis3::Z].dimensions.len()).collect(),
            structure,
            ..Self::default()
        }
    }

    /// Returns this pivot table with the given `title`.
    ///
    /// The title is displayed above the pivot table.  Every pivot table should
    /// have a title.
    pub fn with_title(mut self, title: impl Into<Value>) -> Self {
        self.metadata.title = Some(Box::new(title.into()));
        self.style.show_title = true;
        self
    }

    /// Returns this pivot table with the given `title`, if it is non-`None`.
    pub fn with_optional_title(self, title: Option<Value>) -> Self {
        if let Some(title) = title {
            self.with_title(title)
        } else {
            self
        }
    }

    /// Returns this pivot table with the given `caption`.
    ///
    /// The caption is displayed below the pivot table.  Captions are optional.
    pub fn with_caption(mut self, caption: impl Into<Value>) -> Self {
        self.metadata.caption = Some(Box::new(caption.into()));
        self.style.show_caption = true;
        self
    }

    /// Returns this pivot table with the given `caption`, if it is non-`None`.
    pub fn with_optional_caption(self, caption: Option<Value>) -> Self {
        if let Some(caption) = caption {
            self.with_caption(caption)
        } else {
            self
        }
    }

    /// Returns this pivot table with the given `corner_text`.
    ///
    /// The corner text is displayed in the top-left corner of the pivot table,
    /// above the row headings and to the left of the column headings.  The
    /// space used by corner text can also be used for [Dimension] titles.
    pub fn with_corner_text(mut self, corner_text: impl Into<Value>) -> Self {
        self.metadata.corner_text = Some(Box::new(corner_text.into()));
        self
    }

    /// Returns this pivot table with the given `footnotes`.
    pub fn with_footnotes(mut self, footnotes: Footnotes) -> Self {
        debug_assert!(self.footnotes.is_empty());
        self.footnotes = footnotes;
        self
    }

    /// Returns this pivot table with the given `look`.
    pub fn with_look(self, look: Arc<Look>) -> Self {
        Self {
            style: self.style.with_look(look),
            ..self
        }
    }

    /// Returns this pivot table with the given `style`.
    pub fn with_style(self, style: PivotTableStyle) -> Self {
        Self { style, ..self }
    }

    /// Returns this pivot table with the given `metadata`.
    pub fn with_metadata(self, metadata: PivotTableMetadata) -> Self {
        Self { metadata, ..self }
    }

    /// Returns this pivot table with the given `subtype`.
    ///
    /// A subtype is a locale-invariant command ID for the particular kind of
    /// output that this table represents in the procedure.  This can be the
    /// same as the command name, e.g. `Frequencies`, or different, e.g. `Case
    /// Processing Summary`.
    ///
    /// `Notes` and `Warnings` are common generic subtypes.
    pub fn with_subtype(self, subtype: impl Into<Value>) -> Self {
        Self {
            metadata: self.metadata.with_subtype(subtype),
            ..self
        }
    }

    /// Returns this pivot table with the given `show_values`.
    pub fn with_show_values(self, show_values: Option<Show>) -> Self {
        Self {
            style: self.style.with_show_values(show_values),
            ..self
        }
    }

    /// Returns this pivot table with the given `show_variables`.
    pub fn with_show_variables(self, show_variables: Option<Show>) -> Self {
        Self {
            style: self.style.with_show_variables(show_variables),
            ..self
        }
    }

    /// Returns this pivot table with the given `show_title`.
    pub fn with_show_title(self, show_title: bool) -> Self {
        Self {
            style: self.style.with_show_title(show_title),
            ..self
        }
    }

    /// Returns this pivot table with the given `show_caption`.
    pub fn with_show_caption(self, show_caption: bool) -> Self {
        Self {
            style: self.style.with_show_caption(show_caption),
            ..self
        }
    }

    /// Returns this pivot table with its [Look] modified to show empty rows and
    /// columns.
    pub fn with_show_empty(mut self) -> Self {
        if self.style.look.hide_empty {
            self.look_mut().hide_empty = false;
        }
        self
    }

    /// Returns this pivot table with its [Look] modified to hide empty rows and
    /// columns.
    pub fn with_hide_empty(mut self) -> Self {
        if !self.style.look.hide_empty {
            self.look_mut().hide_empty = true;
        }
        self
    }

    /// Returns this pivot table with the current layer set to `layer`.
    ///
    /// The caller might want to modify the table's look to print just a single
    /// layer.
    pub fn with_layer(mut self, layer: &[usize]) -> Self {
        assert!(self.is_valid_layer(layer));
        self.layer.clear();
        self.layer.extend_from_slice(layer);
        self
    }

    /// Returns true if `layer` is a valid argument for
    /// [PivotTable::with_layer].
    pub fn is_valid_layer(&mut self, layer: &[usize]) -> bool {
        let layer_dimensions = self.axis_dimensions(Axis3::Z);
        layer.len() == layer_dimensions.len()
            && zip(layer, layer_dimensions).all(|(value, dimension)| *value < dimension.len())
    }

    /// Returns this pivot table set to print all layers.
    pub fn with_all_layers(mut self) -> Self {
        if !self.style.look.print_all_layers {
            self.look_mut().print_all_layers = true;
        }
        self
    }

    /// Returns this pivot table with the specified decimal point.
    pub fn with_decimal(mut self, decimal: Decimal) -> Self {
        self.style.settings.decimal = decimal;
        self
    }

    /// Returns this pivot table with the date set as specified.
    pub fn with_date(mut self, date: Option<NaiveDateTime>) -> Self {
        self.metadata.date = date;
        self
    }

    /// Inserts `number` into the cell with the given `data_indexes`, drawing
    /// its format from `class`.
    pub fn insert_number(&mut self, data_indexes: &[usize], number: Option<f64>, class: Class) {
        let format = match class {
            Class::Other => Settings::global().default_format,
            Class::Integer => F40,
            Class::Correlations => F40_3,
            Class::Significance => F40_3,
            Class::Percent => PCT40_1,
            Class::Residual => F40_2,
            Class::Count => F40, // XXX
        };
        let value = Value::new_number(number).with_format(match class {
            Class::Other => ValueFormat::SmallE(format),
            _ => ValueFormat::Other(format),
        });
        self.insert(data_indexes, value);
    }

    /// Returns an iterator for all the values along `axis`.
    fn axis_values(&self, axis: Axis3) -> AxisIterator {
        self.structure.axis_values(axis)
    }

    fn axis_extent(&self, axis: Axis3) -> usize {
        self.structure.axis_extent(axis)
    }

    /// Returns the indexes for the layer to be printed or display on-screen.
    pub fn layer(&self) -> &[usize] {
        &self.layer
    }

    /// Returns the pivot table's dimensions.
    pub fn dimensions(&self) -> &[Dimension] {
        &self.structure.dimensions
    }

    /// Returns the pivot table's axes.
    pub fn axes(&self) -> &EnumMap<Axis3, Axis> {
        &self.structure.axes
    }

    /// Returns the pivot table's [Look], for modification.
    pub fn look_mut(&mut self) -> &mut Look {
        self.style.look_mut()
    }

    /// Returns a label for the table, which is either its title or a default
    /// string.
    pub fn label(&self) -> String {
        match &self.metadata.title {
            Some(title) => title.display(self).to_string(),
            None => String::from("Table"),
        }
    }

    /// Returns the table's title, or an empty value if it doesn't have one.
    pub fn title(&self) -> &Value {
        match &self.metadata.title {
            Some(title) => title,
            None => Value::static_empty(),
        }
    }

    /// Returns the table's subtype, or an empty value if it doesn't have one.
    pub fn subtype(&self) -> &Value {
        match &self.metadata.subtype {
            Some(subtype) => subtype,
            None => Value::static_empty(),
        }
    }

    /// Returns the `HashMap` for cells.  Indexes in the map can be computed
    /// with [cell_index](Self::cell_index).
    pub fn cells(&self) -> &HashMap<usize, Value> {
        &self.cells
    }

    /// Returns the number of cells in the pivot table, which is the product of
    /// the number of categories in each dimension.  (If any of the dimensions
    /// have zero categories, or if the table has no dimensions, the result is zero.)
    pub fn n_cells(&self) -> usize {
        self.structure.n_cells()
    }

    /// Computes an index into the `cells` `HashMap` for `cell_index`.
    pub fn cell_index<C>(&self, cell_index: C) -> usize
    where
        C: CellIndex,
    {
        cell_index.cell_index(self.dimensions().iter().map(|d| d.len()))
    }

    /// Inserts a cell with the given `value` and `cell_index`.
    pub fn insert<C>(&mut self, cell_index: C, value: impl Into<Value>)
    where
        C: CellIndex,
    {
        self.cells.insert(self.cell_index(cell_index), value.into());
    }

    /// Returns the cell with the given `cell_index`, if there is one.
    pub fn get<C>(&self, cell_index: C) -> Option<&Value>
    where
        C: CellIndex,
    {
        self.cells.get(&self.cell_index(cell_index))
    }

    /// Returns the pivot table with cell indexes and values from `iter`
    /// inserted as data.
    pub fn with_data<C>(mut self, iter: impl IntoIterator<Item = (C, Value)>) -> Self
    where
        C: CellIndex,
    {
        self.extend(iter);
        self
    }

    /// Converts per-axis presentation-order indexes in `presentation_indexes`,
    /// into data indexes for each dimension.
    fn convert_indexes_ptod(
        &self,
        presentation_indexes: EnumMap<Axis3, &[usize]>,
    ) -> SmallVec<[usize; 4]> {
        let mut data_indexes = SmallVec::from_elem(0, self.dimensions().len());
        for (axis, presentation_indexes) in presentation_indexes {
            for (&dim_index, &pindex) in self.structure.axes[axis]
                .dimensions
                .iter()
                .zip(presentation_indexes.iter())
            {
                data_indexes[dim_index] = self.structure.dimensions[dim_index].ptod[pindex];
            }
        }
        data_indexes
    }

    /// Returns an iterator for the layer axis:
    ///
    /// - If `print` is true and `self.look.print_all_layers`, then the iterator
    ///   will visit all values of the layer axis.
    ///
    /// - Otherwise, the iterator will just visit `self.current_layer`.
    pub fn layers(&self, print: bool) -> Box<dyn Iterator<Item = SmallVec<[usize; 4]>>> {
        if print && self.style.look.print_all_layers {
            Box::new(self.axis_values(Axis3::Z))
        } else {
            Box::new(once(SmallVec::from_slice(&self.layer)))
        }
    }

    /// Transposes row and columns.
    pub fn transpose(&mut self) {
        self.structure.axes.swap(Axis3::X, Axis3::Y);
    }

    /// Returns an iterator through dimensions on the given `axis`.
    pub fn axis_dimensions(
        &self,
        axis: Axis3,
    ) -> impl DoubleEndedIterator<Item = &Dimension> + ExactSizeIterator {
        self.structure.axis_dimensions(axis)
    }

    fn find_dimension(&self, dim_index: usize) -> Option<(Axis3, usize)> {
        debug_assert!(dim_index < self.structure.dimensions.len());
        for axis in enum_iterator::all::<Axis3>() {
            for (position, dimension) in self.axes()[axis].dimensions.iter().copied().enumerate() {
                if dimension == dim_index {
                    return Some((axis, position));
                }
            }
        }
        None
    }

    /// Moves dimension with index `dim_index` from its current axis to
    /// `new_axis` in position `new_position`.
    ///
    /// `dim_index` is an overall dimension index, in the order passed to
    /// [PivotTable::new] and returned by [PivotTable::dimensions].  This method
    /// doesn't change these indexes.
    ///
    /// `new_position` is an index within `new_axis`, in the range `0..n` where
    /// `n` is the final number of dimensions along that axis.
    ///
    /// # Panic
    ///
    /// Panics if `dim_index` or `new_position` is outside the valid range.
    pub fn move_dimension(&mut self, dim_index: usize, new_axis: Axis3, new_position: usize) {
        let (old_axis, old_position) = self.find_dimension(dim_index).unwrap();
        if old_axis == new_axis && old_position == new_position {
            return;
        }

        // Update the current layer, if necessary.  If we're moving within the
        // layer axis, preserve the current layer.
        match (old_axis, new_axis) {
            (Axis3::Z, Axis3::Z) => {
                // Rearrange the layer axis.
                if old_position < new_position {
                    self.layer[old_position..=new_position].rotate_left(1);
                } else {
                    self.layer[new_position..=old_position].rotate_right(1);
                }
            }
            (Axis3::Z, _) => {
                // A layer is becoming a row or column.
                self.layer.remove(old_position);
            }
            (_, Axis3::Z) => {
                // A row or column is becoming a layer.
                self.layer.insert(new_position, 0);
            }
            _ => (),
        }

        self.structure.axes[old_axis]
            .dimensions
            .remove(old_position);
        self.structure.axes[new_axis]
            .dimensions
            .insert(new_position, dim_index);
    }
}

impl From<&PivotTable> for ValueOptions {
    fn from(value: &PivotTable) -> Self {
        ValueOptions {
            show_values: value.style.show_values,
            show_variables: value.style.show_variables,
            small: value.style.small,
            footnote_marker_type: value.style.look.footnote_marker_type,
            settings: value.style.settings.clone().with_leading_zero_pct(true),
        }
    }
}

/// A dimension.
///
/// A [Dimension] identifies the categories associated with a single dimension
/// within a multidimensional pivot table.
///
/// A dimension contains a collection of categories, which are the leaves in a
/// tree of groups.
///
/// (A dimension or a group can contain zero categories, but this is unusual.
/// If a dimension contains no categories, then its table cannot contain any
/// data.)
#[derive(Clone, Debug, Serialize)]
pub struct Dimension {
    /// Hierarchy of categories within the dimension.  The groups and categories
    /// are sorted in the order that should be used for display.  This might be
    /// different from the original order produced for output if the user
    /// adjusted it.
    ///
    /// The root must always be a group, although it is allowed to have no
    /// subcategories.
    root: Group,

    /// Maps from an index in presentation order to a data index ("p" to "d").
    ///
    /// This is a permutation of `0..n` where `n` is the number of leaves.
    /// Given a [Leaf] that can be found as via `dimension.nth_leaf(leaf_idx)`,
    /// the corresponding data index is `ptod[leaf_idx]`.
    ptod: Vec<usize>,

    /// Display.
    pub hide_all_labels: bool,
}

/// A vector of references to [Group]s.
///
/// Used to represent a sequence of groups along a [Path].  This is a [SmallVec]
/// because groups are usually not deeply nested.
pub type GroupVec<'a> = SmallVec<[&'a Group; 4]>;

/// A path from the root of a [Dimension] to a [Leaf].
pub struct Path<'a> {
    /// Groups along the path.
    ///
    /// There will be at least one group along any valid path, because the
    /// dimension itself is a group.
    pub groups: GroupVec<'a>,

    /// The leaf.
    ///
    /// This is a child of the last [Group].
    pub leaf: &'a Leaf,
}

impl<'a> Path<'a> {
    /// Breaks the path into a vector of [Group]s and a [Leaf].
    pub fn into_parts(self) -> (GroupVec<'a>, &'a Leaf) {
        (self.groups, self.leaf)
    }
}

/// Group indexes visited along a [Path].
pub type IndexVec = SmallVec<[usize; 4]>;

/// Indicates that the argument to [Dimension::set_ptod] was not a valid
/// permutation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct InvalidPermutation;

impl Dimension {
    /// Constructs a new [Dimension] with the given `root`.
    pub fn new(root: Group) -> Self {
        Dimension {
            ptod: (0..root.len()).collect(),
            root,
            hide_all_labels: false,
        }
    }

    /// Returns the root [Group] of the dimension.
    pub fn root(&self) -> &Group {
        &self.root
    }

    /// Returns the presentation-to-data index mapping.
    pub fn ptod(&self) -> &[usize] {
        &self.ptod
    }

    /// Returns this dimension with its presentation-to-data index mapping
    /// replaced by `ptod`, which must be a permutation of `0..self.len()`.
    pub fn set_ptod(&mut self, ptod: Vec<usize>) -> Result<(), InvalidPermutation> {
        if ptod.len() != self.ptod.len() {
            return Err(InvalidPermutation);
        }

        let mut seen = vec![false; ptod.len()];
        for element in ptod.iter().copied() {
            if element >= ptod.len() || seen[element] {
                return Err(InvalidPermutation);
            }
            seen[element] = true;
        }

        self.ptod = ptod;
        Ok(())
    }

    /// Returns this dimension with [Dimension::hide_all_labels] set to true.
    pub fn with_all_labels_hidden(self) -> Self {
        self.with_hide_all_labels(true)
    }

    /// Returns this dimension with [Dimension::hide_all_labels] set to
    /// `hide_all_labels`.
    pub fn with_hide_all_labels(self, hide_all_labels: bool) -> Self {
        Self {
            hide_all_labels,
            ..self
        }
    }

    /// Returns true if [Dimension] has no leaf categories.
    ///
    /// A dimension without leaf categories might still contain a hierarchy of
    /// groups, just none of them with leaves.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of leaf categories in this dimension.
    pub fn len(&self) -> usize {
        self.root.len()
    }

    /// Returns the leaf with the given 0-based `index`, or `None` if `index >=
    /// self.len()`.
    pub fn nth_leaf(&self, index: usize) -> Option<&Leaf> {
        self.root.nth_leaf(index)
    }

    /// Returns the path to the leaf with the given 0-based `index`, or `None`
    /// if `index >= self.len()`.
    pub fn leaf_path(&self, index: usize) -> Option<Path<'_>> {
        self.root.leaf_path(index, SmallVec::new())
    }

    /// Returns the series of group child indexes followed to the leaf with the
    /// given 0-based `index`, or `None` if `index >= self.len()`.
    pub fn index_path(&self, index: usize) -> Option<IndexVec> {
        self.root.index_path(index, SmallVec::new())
    }
}

/// Specifies how to find a [Category] within a [Group].
#[derive(Copy, Clone, Debug)]
pub struct CategoryLocator {
    /// The index of the leaf to start from.
    pub leaf_index: usize,

    /// The number of times to go up a level from the leaf.  If this category is
    /// a leaf, this is 0, otherwise it is positive.
    pub level: usize,
}

impl CategoryLocator {
    /// Constructs a `CategoryLocator` for the leaf with the given 0-based
    /// index.
    pub fn new_leaf(leaf_index: usize) -> Self {
        Self {
            leaf_index,
            level: 0,
        }
    }

    /// Returns a `CategoryLocator` for the parent category of this one.
    pub fn parent(&self) -> Self {
        Self {
            leaf_index: self.leaf_index,
            level: self.level + 1,
        }
    }

    /// If this is a leaf, returns its 0-based index; otherwise, returns `None`.
    pub fn as_leaf(&self) -> Option<usize> {
        (self.level == 0).then_some(self.leaf_index)
    }
}

/// A group of categories within a dimension.
#[derive(Clone, Debug, Serialize)]
pub struct Group {
    /// Number of leaves contained by this group.
    #[serde(skip)]
    len: usize,

    /// The label displayed for the group.
    name: Box<Value>,

    /// Whether to show the label.
    pub show_label: bool,

    /// The child categories.
    ///
    /// A group usually has multiple children, but it is allowed to have
    /// only one or even (pathologically) none.
    children: Vec<Category>,
}

impl Group {
    /// Constructs a new `Group` with the given name.  The group initially has
    /// no children.
    pub fn new(name: impl Into<Value>) -> Self {
        Self::with_capacity(name, 0)
    }

    /// Returns the group's child categories.
    pub fn children(&self) -> &[Category] {
        &self.children
    }

    /// Constructs a new `Group` with the given name and initial child
    /// `capacity`.  The group initially has no children.
    pub fn with_capacity(name: impl Into<Value>, capacity: usize) -> Self {
        Self {
            len: 0,
            name: Box::new(name.into()),
            children: Vec::with_capacity(capacity),
            show_label: false,
        }
    }

    /// Appends `child` to the group's categories.
    pub fn push(&mut self, child: impl Into<Category>) {
        let mut child = child.into();
        if let Some(group) = child.as_group_mut() {
            group.show_label = true;
        }
        self.len += child.len();
        self.children.push(child);
    }

    /// Returns this group with the given `child` appended to its categories.
    pub fn with(mut self, child: impl Into<Category>) -> Self {
        self.push(child);
        self
    }

    /// Returns this group with the given `children` appended to its categories.
    pub fn with_multiple<C>(mut self, children: impl IntoIterator<Item = C>) -> Self
    where
        C: Into<Category>,
    {
        self.extend(children);
        self
    }

    /// Returns this group with `show_label` set to true.
    pub fn with_label_shown(self) -> Self {
        self.with_show_label(true)
    }

    /// Returns this group with `show_label` set as specified.
    pub fn with_show_label(mut self, show_label: bool) -> Self {
        self.show_label = show_label;
        self
    }

    /// Returns the leaf with the given 0-based `index`, or `None` if `index >=
    /// self.len()`.
    fn nth_leaf(&self, mut index: usize) -> Option<&Leaf> {
        for child in &self.children {
            let len = child.len();
            if index < len {
                return child.nth_leaf(index);
            }
            index -= len;
        }
        None
    }

    /// Returns the path to the leaf with the given 0-based `index`, or `None`
    /// if `index >= self.len()`.  `groups` must be the groups already traversed
    /// to arrive at this group.
    fn leaf_path<'a>(&'a self, mut index: usize, mut groups: GroupVec<'a>) -> Option<Path<'a>> {
        for child in &self.children {
            let len = child.len();
            if index < len {
                groups.push(self);
                return child.leaf_path(index, groups);
            }
            index -= len;
        }
        None
    }

    fn index_path(&self, mut index: usize, mut path: IndexVec) -> Option<IndexVec> {
        for (i, child) in self.children.iter().enumerate() {
            let len = child.len();
            if index < len {
                path.push(i);
                return child.index_path(index, path);
            }
            index -= len;
        }
        None
    }

    fn locator_path(&self, locator: CategoryLocator) -> Option<IndexVec> {
        let mut path = self.index_path(locator.leaf_index, IndexVec::new())?;
        path.truncate(path.len().checked_sub(locator.level)?);
        Some(path)
    }

    /// Returns the category corresponding to `locator`.  Returns `None` if
    /// `locator` is invalid (that is, if `locator.leaf_idx >= self.len` or
    /// `locator.level` is greater than the depth of the leaf) or if `locator`
    /// designates `self`.
    pub fn category(&self, locator: CategoryLocator) -> Option<&Category> {
        let path = self.locator_path(locator)?;
        let mut this = &self.children[*path.get(0)?];
        for index in path[1..].iter().copied() {
            this = &this.as_group().unwrap().children[index];
        }
        Some(this)
    }

    /// Returns the category corresponding to `locator`.  Returns `None` if
    /// `locator` is invalid (that is, if `locator.leaf_idx >= self.len` or
    /// `locator.level` is greater than the depth of the leaf) or if `locator`
    /// designates `self`.
    pub fn category_mut(&mut self, locator: CategoryLocator) -> Option<&mut Category> {
        let path = self.locator_path(locator)?;
        let mut this = &mut self.children[*path.get(0)?];
        for index in path[1..].iter().copied() {
            this = &mut this.as_group_mut().unwrap().children[index];
        }
        Some(this)
    }

    /// Returns the number of leaves contained within this group.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if this group contains no leaves.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns this group's label.
    pub fn name(&self) -> &Value {
        &self.name
    }
}

impl<C> Extend<C> for Group
where
    C: Into<Category>,
{
    fn extend<T: IntoIterator<Item = C>>(&mut self, children: T) {
        let children = children.into_iter();
        self.children.reserve(children.size_hint().0);
        for child in children {
            self.push(child);
        }
    }
}

/// A collection of [Footnote]s for a [PivotTable].
///
/// Any [Value] in a pivot table can refer to a footnote.  All of the footnotes
/// used in any [Value] within a given pivot table must be collected into the
/// [Footnotes] attached to the pivot table.  (Footnotes used but not collected
/// might not display as intended, but it's not a safety issue.)
#[derive(Clone, Debug, Default, Serialize)]
pub struct Footnotes(Vec<Arc<Footnote>>);

impl Footnotes {
    /// Constructs a new, empty collection of footnotes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of footnotes in the collection.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the collection contains no footnotes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Adds `footnote` to the collection.
    pub fn push(&mut self, footnote: Footnote) -> Arc<Footnote> {
        let footnote = Arc::new(footnote.with_index(self.0.len()));
        self.0.push(footnote.clone());
        footnote
    }

    /// Returns the footnote with 0-based index `index`, or `None` if `index >=
    /// self.len()`.
    pub fn get(&self, index: usize) -> Option<&Arc<Footnote>> {
        self.0.get(index)
    }
}

impl Index<usize> for Footnotes {
    type Output = Arc<Footnote>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl<'a> IntoIterator for &'a Footnotes {
    type Item = &'a Arc<Footnote>;

    type IntoIter = std::slice::Iter<'a, Arc<Footnote>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl FromIterator<Footnote> for Footnotes {
    fn from_iter<T: IntoIterator<Item = Footnote>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .enumerate()
                .map(|(index, footnote)| Arc::new(footnote.with_index(index)))
                .collect(),
        )
    }
}

/// A leaf category within a [Group] and ultimately within a [Dimension].
#[derive(Clone, Debug)]
pub struct Leaf {
    data_index: usize,
    name: Box<Value>,
}

impl Leaf {
    /// Constructs a new `Leaf` with the given label.
    pub fn new(name: Value) -> Self {
        Self {
            data_index: 0,
            name: Box::new(name),
        }
    }

    /// Returns the leaf's label.
    pub fn name(&self) -> &Value {
        &self.name
    }

    /// Returns a sequence of `Leaf`s for the given range, for use with
    /// [Group::with_multiple] or [Group::extend].
    ///
    /// This is useful for constructing dimensions that aren't meant to be
    /// shown.
    pub fn numbers(range: Range<usize>) -> impl Iterator<Item = Leaf> {
        range.map(|i| Self::new(Value::new_integer(Some(i as f64))))
    }
}

impl Serialize for Leaf {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.name.serialize(serializer)
    }
}

/// Pivot result classes.
///
/// Used by [PivotTable::insert_number].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Class {
    /// An integer value.
    Integer,
    /// A correlation value.
    Correlations,
    /// A measure of significance.
    Significance,
    /// A percentage.
    Percent,
    /// A residual value.
    Residual,
    /// A count.
    ///
    /// With a weighted dataset, these are by default displayed with the weight
    /// variable's format.
    Count,
    /// A value that doesn't fit in one of the other categories.
    Other,
}

/// A leaf category or a group of them.
#[derive(Clone, Debug, Serialize)]
pub enum Category {
    /// A group.
    Group(
        /// The group.
        Group,
    ),
    /// A leaf.
    Leaf(
        /// The leaf.
        Leaf,
    ),
}

impl Category {
    /// Returns the [Group] in this `Category`, if there is one.
    pub fn as_group(&self) -> Option<&Group> {
        match self {
            Category::Group(group) => Some(group),
            Category::Leaf(_) => None,
        }
    }

    /// Returns the [Group] in this `Category`, if there is one, for
    /// modification.
    pub fn as_group_mut(&mut self) -> Option<&mut Group> {
        match self {
            Category::Group(group) => Some(group),
            Category::Leaf(_) => None,
        }
    }

    /// Returns the [Leaf] in this `Category`, if there is one.
    pub fn as_leaf(&self) -> Option<&Leaf> {
        match self {
            Category::Leaf(leaf) => Some(leaf),
            Category::Group(_) => None,
        }
    }

    /// Returns the [Leaf] in this `Category`, if there is one, for
    /// modification.
    pub fn as_leaf_mut(&mut self) -> Option<&mut Leaf> {
        match self {
            Category::Leaf(leaf) => Some(leaf),
            Category::Group(_) => None,
        }
    }

    /// Returns the category's label.
    pub fn name(&self) -> &Value {
        match self {
            Category::Group(group) => &group.name,
            Category::Leaf(leaf) => &leaf.name,
        }
    }

    /// Returns the category's label, for modification.
    pub fn name_mut(&mut self) -> &mut Value {
        match self {
            Category::Group(group) => &mut group.name,
            Category::Leaf(leaf) => &mut leaf.name,
        }
    }

    /// Returns true if this category's label should be shown.
    ///
    /// A leaf category's label is always shown, unless
    /// [Dimension::hide_all_labels] is true.
    pub fn show_label(&self) -> bool {
        match self {
            Category::Group(group) => group.show_label,
            Category::Leaf(_) => true,
        }
    }
    /// Returns true if the category contains no leaves.
    ///
    /// If this is a leaf category, this always returns false.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of leaves that this category contains.
    pub fn len(&self) -> usize {
        match self {
            Category::Group(group) => group.len,
            Category::Leaf(_) => 1,
        }
    }

    fn nth_leaf(&self, index: usize) -> Option<&Leaf> {
        match self {
            Category::Group(group) => group.nth_leaf(index),
            Category::Leaf(leaf) if index == 0 => Some(leaf),
            _ => None,
        }
    }

    fn leaf_path<'a>(&'a self, index: usize, groups: GroupVec<'a>) -> Option<Path<'a>> {
        match self {
            Category::Group(group) => group.leaf_path(index, groups),
            Category::Leaf(leaf) if index == 0 => Some(Path { groups, leaf }),
            _ => None,
        }
    }

    fn index_path(&self, index: usize, path: IndexVec) -> Option<IndexVec> {
        match self {
            Category::Group(group) => group.index_path(index, path),
            Category::Leaf(_) if index == 0 => Some(path),
            _ => None,
        }
    }

    fn locator_path(&self, locator: CategoryLocator) -> Option<IndexVec> {
        let mut path = self.index_path(locator.leaf_index, IndexVec::new())?;
        path.truncate(path.len().checked_sub(locator.level)?);
        Some(path)
    }

    /// Returns `None` if `locator` is invalid (that is, if `locator.leaf_idx >=
    /// self.len` or `locator.level` is greater than the depth of the leaf).
    fn category(&self, locator: CategoryLocator) -> Option<&Category> {
        let mut this = self;
        for index in this.locator_path(locator)? {
            this = &this.as_group().unwrap().children[index];
        }
        Some(this)
    }

    fn category_mut(&mut self, locator: CategoryLocator) -> Option<&mut Category> {
        let mut this = self;
        for index in this.locator_path(locator)? {
            this = &mut this.as_group_mut().unwrap().children[index];
        }
        Some(this)
    }
}

impl From<Group> for Category {
    fn from(group: Group) -> Self {
        Self::Group(group)
    }
}

impl From<Leaf> for Category {
    fn from(group: Leaf) -> Self {
        Self::Leaf(group)
    }
}

impl From<Value> for Category {
    fn from(name: Value) -> Self {
        Leaf::new(name).into()
    }
}

impl From<&Variable> for Category {
    fn from(variable: &Variable) -> Self {
        Value::new_variable(variable).into()
    }
}

impl From<&str> for Category {
    fn from(name: &str) -> Self {
        Self::Leaf(Leaf::new(Value::new_text(name)))
    }
}

impl From<String> for Category {
    fn from(name: String) -> Self {
        Self::Leaf(Leaf::new(Value::new_text(name)))
    }
}

impl From<&String> for Category {
    fn from(name: &String) -> Self {
        Self::Leaf(Leaf::new(Value::new_text(name)))
    }
}

/// An axis of a 2-dimensional table.
#[derive(Copy, Clone, Debug, Enum, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis2 {
    /// X axis.
    X,
    /// Y axis.
    Y,
}

impl Axis2 {
    /// Returns a map from [Axis2::X] to `x` and from [Axis2::Y] to `y`.
    pub fn new_enum<T>(x: T, y: T) -> EnumMap<Axis2, T> {
        EnumMap::from_array([x, y])
    }

    /// Returns the axis's name, in lowercase, as a static string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Axis2::X => "x",
            Axis2::Y => "y",
        }
    }
}

impl Display for Axis2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Not for Axis2 {
    type Output = Self;

    fn not(self) -> Self::Output {
        match self {
            Self::X => Self::Y,
            Self::Y => Self::X,
        }
    }
}

/// Error converting [Axis3::Z] to [Axis2].
#[derive(Copy, Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Can't convert `Axis3::Z` to `Axis2`.")]
pub struct ZAxis;

impl TryFrom<Axis3> for Axis2 {
    type Error = ZAxis;

    fn try_from(value: Axis3) -> Result<Self, Self::Error> {
        match value {
            Axis3::X => Ok(Axis2::X),
            Axis3::Y => Ok(Axis2::Y),
            Axis3::Z => Err(ZAxis),
        }
    }
}

/// A 2-dimensional `(x,y)` pair.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Coord2(pub EnumMap<Axis2, isize>);

impl Coord2 {
    /// Constructs a new `Coord2` with the given `x` and `y` coordinates.
    pub fn new(x: isize, y: isize) -> Self {
        use Axis2::*;
        Self(enum_map! {
            X => x,
            Y => y
        })
    }

    /// Constructs a new `Coord2` with coordinate `az` on axis `a` and
    /// coordinate `bz` on the other axis.
    pub fn for_axis((a, az): (Axis2, isize), bz: isize) -> Self {
        let mut coord = Self::default();
        coord[a] = az;
        coord[!a] = bz;
        coord
    }

    /// Constructs a new `Coord2` with coordinates from function `f`.
    pub fn from_fn<F>(f: F) -> Self
    where
        F: FnMut(Axis2) -> isize,
    {
        Self(EnumMap::from_fn(f))
    }

    /// Returns the X coordinate.
    pub fn x(&self) -> isize {
        self.0[Axis2::X]
    }

    /// Returns the Y coordinate.
    pub fn y(&self) -> isize {
        self.0[Axis2::Y]
    }

    /// Returns the coordinate on the given `axis`.
    pub fn get(&self, axis: Axis2) -> isize {
        self.0[axis]
    }
}

impl From<EnumMap<Axis2, isize>> for Coord2 {
    fn from(value: EnumMap<Axis2, isize>) -> Self {
        Self(value)
    }
}

impl Index<Axis2> for Coord2 {
    type Output = isize;

    fn index(&self, index: Axis2) -> &Self::Output {
        &self.0[index]
    }
}

impl IndexMut<Axis2> for Coord2 {
    fn index_mut(&mut self, index: Axis2) -> &mut Self::Output {
        &mut self.0[index]
    }
}

/// A 2-dimensional rectangle.
#[derive(Clone, Debug, Default)]
pub struct Rect2(
    /// The range along each axis.
    pub EnumMap<Axis2, Range<isize>>,
);

impl Rect2 {
    /// Constructs a new `Rect2` that covers the given X and Y ranges.
    pub fn new(x_range: Range<isize>, y_range: Range<isize>) -> Self {
        Self(enum_map! {
            Axis2::X => x_range.clone(),
            Axis2::Y => y_range.clone(),
        })
    }

    /// Construct a new `Rect2` that covers `a_range` along axis `a` and
    /// `b_range` along the other axis.
    pub fn for_ranges((a, a_range): (Axis2, Range<isize>), b_range: Range<isize>) -> Self {
        let b = !a;
        let mut ranges = EnumMap::default();
        ranges[a] = a_range;
        ranges[b] = b_range;
        Self(ranges)
    }

    /// Returns the top-left corner of the rectangle.
    pub fn top_left(&self) -> Coord2 {
        use Axis2::*;
        Coord2::new(self[X].start, self[Y].start)
    }

    /// Construct a new `Rect2` that covers the ranges returned by `f`.
    pub fn from_fn<F>(f: F) -> Self
    where
        F: FnMut(Axis2) -> Range<isize>,
    {
        Self(EnumMap::from_fn(f))
    }

    /// Shifts the rectangle by `offset` along its axes.
    pub fn translate(self, offset: Coord2) -> Rect2 {
        Self::from_fn(|axis| self[axis].start + offset[axis]..self[axis].end + offset[axis])
    }

    /// Returns true if the rectangle is empty.
    pub fn is_empty(&self) -> bool {
        self[Axis2::X].is_empty() || self[Axis2::Y].is_empty()
    }
}

impl From<EnumMap<Axis2, Range<isize>>> for Rect2 {
    fn from(value: EnumMap<Axis2, Range<isize>>) -> Self {
        Self(value)
    }
}

impl Index<Axis2> for Rect2 {
    type Output = Range<isize>;

    fn index(&self, index: Axis2) -> &Self::Output {
        &self.0[index]
    }
}

impl IndexMut<Axis2> for Rect2 {
    fn index_mut(&mut self, index: Axis2) -> &mut Self::Output {
        &mut self.0[index]
    }
}

/// What to show in a footnote marker.
#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FootnoteMarkerType {
    /// a, b, c, ...
    #[default]
    Alphabetic,

    /// 1, 2, 3, ...
    Numeric,
}

/// How to show a footnote marker.
#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FootnoteMarkerPosition {
    /// Subscripts.
    Subscript,

    /// Superscripts.
    #[default]
    Superscript,
}

/// A [Look] and other styling for a [PivotTable].
#[derive(Clone, Debug, Serialize)]
pub struct PivotTableStyle {
    /// The [Look].
    ///
    /// The division between [Look] and the rest of the styling in this
    /// structure is fairly arbitrary.  The ultimate reason for the division is
    /// simply because that's how SPSS documentation and file formats do it.
    pub look: Arc<Look>,

    /// Display inner column labels as vertical text?
    pub rotate_inner_column_labels: bool,

    /// Display outer row labels as vertical text?
    pub rotate_outer_row_labels: bool,

    /// Show grid lines between data cells?
    pub show_grid_lines: bool,

    /// Display the title?
    pub show_title: bool,

    /// Display the caption?
    pub show_caption: bool,

    /// Default [Show] value for showing values in [Value]s that don't specify
    /// their own.
    ///
    /// If this is `None` then a global default is used.
    pub show_values: Option<Show>,

    /// Default [Show] value for showing variables in [Value]s that don't
    /// specify their own.
    ///
    /// If this is `None` then a global default is used.
    pub show_variables: Option<Show>,

    /// Column and row sizing and page breaks:
    ///
    /// - `sizing[Axis2::X]` is sizes for columns.
    /// - `sizing[Axis2::Y]` is sizes for rows.
    pub sizing: EnumMap<Axis2, Option<Box<Sizing>>>,

    /// Format settings.
    pub settings: FormatSettings,

    /// Numeric grouping character (usually `.` or `,`).
    pub grouping: Option<char>,

    /// The threshold for [ValueFormat::SmallE].
    pub small: f64,

    /// The format to use for weight and count variables.
    pub weight_format: Format,
}

impl Default for PivotTableStyle {
    fn default() -> Self {
        Self {
            look: Look::shared_default(),
            rotate_inner_column_labels: false,
            rotate_outer_row_labels: false,
            show_grid_lines: false,
            show_title: true,
            show_caption: true,
            show_values: None,
            show_variables: None,
            sizing: EnumMap::default(),
            settings: Default::default(), // XXX from settings
            grouping: None,
            small: 0.0001, // XXX from settings.
            weight_format: F40,
        }
    }
}

impl PivotTableStyle {
    /// Returns this style with the given `look`.
    pub fn with_look(self, look: Arc<Look>) -> Self {
        Self { look, ..self }
    }

    /// Returns this style with the given `show_values`.
    pub fn with_show_values(self, show_values: Option<Show>) -> Self {
        Self {
            show_values,
            ..self
        }
    }

    /// Returns this style with the given `show_variables`.
    pub fn with_show_variables(self, show_variables: Option<Show>) -> Self {
        Self {
            show_variables,
            ..self
        }
    }

    /// Returns this style with the given `show_title`.
    pub fn with_show_title(self, show_title: bool) -> Self {
        Self { show_title, ..self }
    }

    /// Returns this style with the given `show_caption`.
    pub fn with_show_caption(self, show_caption: bool) -> Self {
        Self {
            show_caption,
            ..self
        }
    }

    /// Returns a mutable reference to this style's [Look].
    ///
    /// This unshares the [Arc] used to refer to the [Look].
    pub fn look_mut(&mut self) -> &mut Look {
        Arc::make_mut(&mut self.look)
    }
}

/// Metadata for a [PivotTable].
#[derive(Clone, Debug, Serialize)]
pub struct PivotTableMetadata {
    /// Title.
    ///
    /// The title is displayed above the table.  Every table should have a
    /// title.
    pub title: Option<Box<Value>>,

    /// Caption.
    ///
    /// The caption is displayed below the table.  Captions are optional.
    pub caption: Option<Box<Value>>,

    /// Corner text, displayed in the top-left corner of the table.  Corner text
    /// is optional.
    pub corner_text: Option<Box<Value>>,

    /// User-specified optional notes, with special variables expanded into
    /// their values.
    pub notes: Option<String>,

    /// User-specified optional notes, with special variables left in their
    /// original forms.
    ///
    /// This allows the notes to be edited in their original form and then
    /// expanded for display.
    pub notes_unexpanded: Option<String>,

    /// The localized name of the command that produced this pivot table,
    /// e.g. `Frequencies` translated into the local language.
    pub command_local: Option<String>,

    /// The locale-invariant name of the command that produced this pivot table,
    /// e.g. `Frequencies`.
    pub command_c: Option<String>,

    /// The locale-invariant command ID for the particular kind of output that
    /// this table represents in the procedure.  This can be the same as
    /// `command_c`, e.g. `Frequencies`, or different, e.g. `Case Processing
    /// Summary`.
    ///
    /// `Notes` and `Warnings` are common generic subtypes.
    pub subtype: Option<Box<Value>>,

    /// The language used in output.
    pub language: Option<String>,

    /// A locale, including an encoding, such as `en_US.windows-1252` or
    /// `it_IT.windows-1252`.
    pub locale: Option<String>,

    /// Name of the dataset analyzed to produce the output, e.g. `DataSet1`.
    pub dataset: Option<String>,

    /// Name of the file that the dataset is from, e.g. `C:\Users\foo\bar.sav`.
    pub datafile: Option<String>,

    /// Creation date for the table.
    pub date: Option<NaiveDateTime>,
}

impl Default for PivotTableMetadata {
    fn default() -> Self {
        Self {
            title: None,
            caption: None,
            corner_text: None,
            notes: None,
            notes_unexpanded: None,
            command_local: None,
            command_c: None,
            subtype: None,
            language: None,
            locale: None,
            dataset: None,
            datafile: None,
            date: Some(Utc::now().naive_local()),
        }
    }
}

impl PivotTableMetadata {
    /// Return this metadata with the given `subtype`.
    pub fn with_subtype(self, subtype: impl Into<Value>) -> Self {
        Self {
            subtype: Some(Box::new(subtype.into())),
            ..self
        }
    }
}

/// A pivot table.
///
/// Pivot tables are PSPP's primary form of output.  They are analogous to the
/// pivot tables you might be familiar with from spreadsheets and databases.
/// See <https://en.wikipedia.org/wiki/Pivot_table> for a brief introduction to
/// the overall concept of a pivot table.
#[derive(Clone, Debug, Serialize)]
pub struct PivotTable {
    /// Style.
    pub style: PivotTableStyle,

    /// Current layer indexes, with `axes[Axis3::Z].dimensions.len()` elements.
    /// `layer[i]` is an offset into
    /// `axes[Axis3::Z].dimensions[i].data_leaves[]`, except that a dimension
    /// can have zero leaves, in which case `layer[i]` is zero and there's no
    /// corresponding leaf.
    layer: Vec<usize>,

    /// Metadata.
    pub metadata: PivotTableMetadata,

    /// Footnotes.
    pub footnotes: Footnotes,

    /// Table structure.
    #[serde(flatten)]
    structure: Structure,

    /// Data.
    cells: HashMap<usize, Value>,
}

impl Default for PivotTable {
    fn default() -> Self {
        Self {
            style: PivotTableStyle::default(),
            metadata: PivotTableMetadata::default(),
            layer: Vec::new(),
            footnotes: Footnotes::new(),
            structure: Structure::default(),
            cells: HashMap::new(),
        }
    }
}

/// The structure of a pivot table.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Structure {
    /// Dimensions.
    dimensions: Vec<Dimension>,

    /// Axes.
    axes: EnumMap<Axis3, Axis>,
}

/// [Structure::new] error for a missing or duplicate dimension among the axes.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct InvalidAxes;

impl Structure {
    /// Creates a new `Structure` from the arguments.
    ///
    /// Each of the `dimensions` must appear exactly once in `axes`.
    ///
    /// Usually [Structure::from] is easier.
    pub fn new(
        dimensions: Vec<Dimension>,
        axes: EnumMap<Axis3, Axis>,
    ) -> Result<Self, InvalidAxes> {
        if dimensions.len()
            != axes
                .values()
                .map(|axis| axis.dimensions.len())
                .sum::<usize>()
        {
            return Err(InvalidAxes);
        }
        let mut seen = vec![false; dimensions.len()];
        for d in axes
            .values()
            .flat_map(|axis| axis.dimensions.iter())
            .copied()
        {
            if let Some(seen) = seen.get_mut(d)
                && !*seen
            {
                *seen = true;
            } else {
                return Err(InvalidAxes);
            }
        }
        Ok(Self { dimensions, axes })
    }

    /// Returns the number of cells in the pivot table, which is the product of
    /// the number of categories in each dimension.  (If any of the dimensions
    /// have zero categories, or if the table has no dimensions, the result is zero.)
    pub fn n_cells(&self) -> usize {
        if self.dimensions.is_empty() {
            0
        } else {
            self.dimensions.iter().map(|d| d.len()).product()
        }
    }

    /// Returns an iterator through dimensions on the given `axis`.
    pub fn axis_dimensions(
        &self,
        axis: Axis3,
    ) -> impl DoubleEndedIterator<Item = &Dimension> + ExactSizeIterator {
        self.axes[axis]
            .dimensions
            .iter()
            .copied()
            .map(|index| &self.dimensions[index])
    }

    /// Returns an iterator for all the values along `axis`.
    fn axis_values(&self, axis: Axis3) -> AxisIterator {
        AxisIterator {
            indexes: smallvec![0; self.axes[axis].dimensions.len()],
            lengths: self.axis_dimensions(axis).map(|d| d.len()).collect(),
            done: self.axis_extent(axis) == 0,
        }
    }

    /// Returns the number of categories along `axis`, that is, the product of
    /// the number of categories in all of the dimensions on `axis`.
    pub fn axis_extent(&self, axis: Axis3) -> usize {
        self.axis_dimensions(axis).map(|d| d.len()).product()
    }
}

impl<T> From<T> for Structure
where
    T: IntoIterator<Item = (Axis3, Dimension)>,
{
    fn from(value: T) -> Self {
        let mut structure = Structure::default();
        for (axis, dimension) in value {
            structure.axes[axis]
                .dimensions
                .push(structure.dimensions.len());
            structure.dimensions.push(dimension);
        }
        structure
    }
}

/// A type that can calculate the index into a [PivotTable]'s data hashmap.
pub trait CellIndex {
    /// Given the pivot table's `dimensions`, returns an index.
    fn cell_index<I>(self, dimensions: I) -> usize
    where
        I: ExactSizeIterator<Item = usize>;
}

impl<T> CellIndex for T
where
    T: AsRef<[usize]>,
{
    fn cell_index<I>(self, dimensions: I) -> usize
    where
        I: ExactSizeIterator<Item = usize>,
    {
        let data_indexes = self.as_ref();
        let mut index = 0;
        for (dimension, data_index) in dimensions.zip_eq(data_indexes.iter()) {
            debug_assert!(*data_index < dimension);
            index = dimension * index + data_index;
        }
        index
    }
}

/// A precomputed index.
pub struct PrecomputedIndex(
    /// The index.
    pub usize,
);

impl CellIndex for PrecomputedIndex {
    fn cell_index<I>(self, _dimensions: I) -> usize
    where
        I: ExactSizeIterator<Item = usize>,
    {
        self.0
    }
}

impl<C> Extend<(C, Value)> for PivotTable
where
    C: CellIndex,
{
    fn extend<T: IntoIterator<Item = (C, Value)>>(&mut self, iter: T) {
        for (cell_index, value) in iter {
            self.insert(cell_index, value);
        }
    }
}

/// A footnote in a [PivotTable].
///
/// A footnote is attached directly to a [Value], but it will only be displayed
/// correctly if it is also added to [PivotTable::footnotes] for its pivot
/// table.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Footnote {
    /// The index within [Footnotes].
    #[serde(skip)]
    index: usize,

    /// The footnote text.
    pub content: Box<Value>,

    /// The footnote marker.
    ///
    /// This is usually `None`, in which case [FootnoteMarkerType] determines
    /// the default marker.
    pub marker: Option<Box<Value>>,

    /// Whether to show the footnote.
    pub show: bool,
}

impl Footnote {
    /// Constructs a new footnote.
    pub fn new(content: impl Into<Value>) -> Self {
        Self {
            index: 0,
            content: Box::new(content.into()),
            marker: None,
            show: true,
        }
    }

    /// Returns the footnote with the given optional marker.
    pub fn with_marker(self, marker: Option<Value>) -> Self {
        Self {
            marker: marker.map(Box::new),
            ..self
        }
    }

    /// Returns the footnote with the given marker.
    pub fn with_some_marker(self, marker: impl Into<Value>) -> Self {
        Self::with_marker(self, Some(marker.into()))
    }

    /// Return the footnote with the given `show`.
    pub fn with_show(self, show: bool) -> Self {
        Self { show, ..self }
    }

    /// Return the footnote with the given `index`.
    pub fn with_index(self, index: usize) -> Self {
        Self { index, ..self }
    }

    /// Returns an object for formatting the footnote's marker.
    pub fn display_marker(&self, options: impl Into<ValueOptions>) -> impl Display {
        DisplayMarker {
            footnote: self,
            options: options.into(),
        }
    }

    /// Returns an object for formatting the footnote's text.
    pub fn display_content(&self, options: impl Into<ValueOptions>) -> impl Display {
        self.content.display(options)
    }

    /// Returns the footnote's index.
    pub fn index(&self) -> usize {
        self.index
    }
}

impl Default for Footnote {
    fn default() -> Self {
        Footnote::new(Value::default())
    }
}

struct DisplayMarker<'a> {
    footnote: &'a Footnote,
    options: ValueOptions,
}

impl Display for DisplayMarker<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(marker) = &self.footnote.marker {
            write!(f, "{}", marker.display(&self.options).without_suffixes())
        } else {
            let i = self.footnote.index + 1;
            match self.options.footnote_marker_type {
                FootnoteMarkerType::Alphabetic => write!(f, "{}", Display26Adic::new_lowercase(i)),
                FootnoteMarkerType::Numeric => write!(f, "{i}"),
            }
        }
    }
}

/// Displays a number in 26adic notation.
///
/// Zero is displayed as the empty string, 1 through 26 as `a` through `z`, 27
/// through 52 as `aa` through `az`, and so on.
pub struct Display26Adic {
    value: usize,
    base: u8,
}

impl Display26Adic {
    /// Constructs a `Display26Adic` for `value`, with letters in lowercase.
    pub fn new_lowercase(value: usize) -> Self {
        Self { value, base: b'a' }
    }

    /// Constructs a `Display26Adic` for `value`, with letters in uppercase.
    pub fn new_uppercase(value: usize) -> Self {
        Self { value, base: b'A' }
    }
}

impl Display for Display26Adic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output = SmallVec::<[u8; 16]>::new();
        let mut number = self.value;
        while number > 0 {
            number -= 1;
            let digit = (number % 26) as u8;
            output.push(digit + self.base);
            number /= 26;
        }
        output.reverse();
        write!(f, "{}", str::from_utf8(&output).unwrap())
    }
}

/// An entry in a metadata table.
pub struct MetadataEntry {
    /// The label for the entry.
    pub name: Value,

    /// The value for the entry.
    pub value: MetadataValue,
}

impl MetadataEntry {
    /// Constructs a new metadata entry with the given name and value.
    pub fn new(name: impl Into<Value>, value: MetadataValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }

    /// Converts the metadata entry into a pivot table.
    pub fn into_pivot_table(self) -> PivotTable {
        let mut data = Vec::new();
        let group = match self.visit(&mut data) {
            Category::Group(group) => group,
            Category::Leaf(leaf) => Group::new("Metadata").with(leaf).with_label_shown(),
        };
        PivotTable::new([(Axis3::Y, Dimension::new(group))]).with_data(
            data.into_iter()
                .enumerate()
                .filter(|(_row, value)| !value.is_empty())
                .map(|(row, value)| ([row], value)),
        )
    }
    fn visit(self, data: &mut Vec<Value>) -> Category {
        match self.value {
            MetadataValue::Leaf(value) => {
                data.push(value);
                Leaf::new(self.name).into()
            }
            MetadataValue::Group(items) => Group::with_capacity(self.name, items.len())
                .with_multiple(items.into_iter().map(|item| item.visit(data)))
                .into(),
        }
    }
}

/// A value in a metadata table.
pub enum MetadataValue {
    /// A value.
    Leaf(
        /// The value.
        Value,
    ),
    /// A nested group of entries.
    Group(
        /// The entries.
        Vec<MetadataEntry>,
    ),
}

impl MetadataValue {
    /// Construct a new "leaf" metadata value.
    pub fn new_leaf(value: impl Into<Value>) -> Self {
        Self::Leaf(value.into())
    }
}

impl Serialize for MetadataValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            MetadataValue::Leaf(value) => value.serialize_bare(serializer),
            MetadataValue::Group(items) => {
                let mut map = serializer.serialize_map(Some(items.len()))?;
                for item in items {
                    let name = item.name.display(()).to_string();
                    map.serialize_entry(&name, &item.value)?;
                }
                map.end()
            }
        }
    }
}
impl Serialize for MetadataEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match &self.value {
            MetadataValue::Leaf(value) => {
                let mut map = serializer.serialize_map(Some(1))?;
                let name = self.name.display(()).to_string();
                map.serialize_entry(&name, &BareValue(value))?;
                map.end()
            }
            MetadataValue::Group(items) => {
                let mut map = serializer.serialize_map(Some(items.len()))?;
                for item in items {
                    let name = item.name.display(()).to_string();
                    map.serialize_entry(&name, &item.value)?;
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use crate::output::pivot::{
        Display26Adic, MetadataEntry, MetadataValue,
        look::Color,
        tests::assert_rendering,
        value::{TemplateValue, Value, ValueInner},
    };

    #[test]
    fn parse_color() {
        assert_eq!(Color::from_str("red"), Ok(Color::new(255, 0, 0)));
        assert_eq!(Color::from_str("transparent"), Ok(Color::TRANSPARENT));
        assert_eq!(Color::from_str("rgb(12,34,56)"), Ok(Color::new(12, 34, 56)));
        assert_eq!(Color::from_str("#abcdef"), Ok(Color::new(0xab, 0xcd, 0xef)));
        assert_eq!(Color::from_str("abcdef"), Ok(Color::new(0xab, 0xcd, 0xef)));
        assert_eq!(Color::from_str("transparent"), Ok(Color::TRANSPARENT));
    }

    #[test]
    fn display_26adic() {
        for (number, lowercase, uppercase) in [
            (0, "", ""),
            (1, "a", "A"),
            (2, "b", "B"),
            (26, "z", "Z"),
            (27, "aa", "AA"),
            (28, "ab", "AB"),
            (29, "ac", "AC"),
            (18278, "zzz", "ZZZ"),
            (18279, "aaaa", "AAAA"),
            (19010, "abcd", "ABCD"),
        ] {
            assert_eq!(Display26Adic::new_lowercase(number).to_string(), lowercase);
            assert_eq!(Display26Adic::new_uppercase(number).to_string(), uppercase);
        }
    }

    #[test]
    fn template() {
        for (template, expected) in [
            (
                "1: [:^1,:]1; [:^1,:]2",
                "1: First,1.00,Second,2,; Third,3.00,Fourth,4,",
            ),
            (r#"2: [:^1\n:]1"#, "2: First\n1.00\nSecond\n2\n"),
            (r#"3: [:^1 = ^2\n:]1"#, "3: First = 1.00\nSecond = 2\n"),
            ("4: [%1:, ^1:]1", "4: First, 1.00, Second, 2"),
            ("5: [%1 = %2:, ^1 = ^2:]1", "5: First = 1.00, Second = 2"),
            ("6: [%1:, ^1:]1", "6: First, 1.00, Second, 2"),
            ("7: [%1:, ^1: and $1]1", "7: First, 1.00, Second and 2"),
            ("8: [%1:, ^1: and $1]3", "8: One and two"),
            ("9: [%1:, ^1: and $1]4", "9: Just one"),
        ] {
            let value = Value::new(ValueInner::Template(TemplateValue {
                args: vec![
                    vec![
                        Value::new_user_text("First"),
                        Value::new_number(Some(1.0)),
                        Value::new_user_text("Second"),
                        Value::new_integer(Some(2.0)),
                    ],
                    vec![
                        Value::new_user_text("Third"),
                        Value::new_number(Some(3.0)),
                        Value::new_user_text("Fourth"),
                        Value::new_integer(Some(4.0)),
                    ],
                    vec![Value::new_user_text("One"), Value::new_user_text("two")],
                    vec![Value::new_user_text("Just one")],
                ],
                localized: String::from(template),
                id: None,
            }));
            assert_eq!(value.display(()).to_string(), expected);
        }
    }

    #[test]
    fn metadata_entry() {
        let tree = MetadataEntry {
            name: Value::from("Group"),
            value: MetadataValue::Group(vec![
                MetadataEntry {
                    name: Value::from("Name 1"),
                    value: MetadataValue::Leaf(Value::from("Value 1")),
                },
                MetadataEntry {
                    name: Value::from("Subgroup 1"),
                    value: MetadataValue::Group(vec![
                        MetadataEntry {
                            name: Value::from("Subname 1"),
                            value: MetadataValue::Leaf(Value::from("Subvalue 1")),
                        },
                        MetadataEntry {
                            name: Value::from("Subname 2"),
                            value: MetadataValue::Leaf(Value::from("Subvalue 2")),
                        },
                        MetadataEntry {
                            name: Value::from("Subname 3"),
                            value: MetadataValue::Leaf(Value::new_integer(Some(3.0))),
                        },
                    ]),
                },
                MetadataEntry {
                    name: Value::from("Name 2"),
                    value: MetadataValue::Leaf(Value::from("Value 2")),
                },
            ]),
        };
        assert_eq!(
            serde_json::to_string_pretty(&tree).unwrap(),
            r#"{
  "Name 1": "Value 1",
  "Subgroup 1": {
    "Subname 1": "Subvalue 1",
    "Subname 2": "Subvalue 2",
    "Subname 3": 3
  },
  "Name 2": "Value 2"
}"#
        );

        assert_rendering("metadata_entry", &tree.into_pivot_table());
    }
}
