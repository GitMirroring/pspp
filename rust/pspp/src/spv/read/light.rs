use std::{
    fmt::Debug,
    io::{Read, Seek},
    iter::repeat,
    ops::Deref,
    str::FromStr,
    sync::Arc,
};

use binrw::{
    BinRead, BinResult, Endian, Error as BinError, VecArgs, binread, error::ContextExt,
    io::TakeSeekExt,
};
use chrono::DateTime;
use displaydoc::Display;
use encoding_rs::{Encoding, WINDOWS_1252};
use enum_map::{EnumMap, enum_map};

use crate::{
    data::Datum,
    format::{
        CustomCurrencies, Decimal, Decimals, Epoch, F40, F40_2, NumberStyle, Settings, Type,
        UncheckedFormat, Width,
    },
    output::pivot::{
        self, Axis, Axis2, Axis3, FootnoteMarkerPosition, FootnoteMarkerType, Footnotes, Group,
        PivotTable, PivotTableMetadata, PivotTableStyle, PrecomputedIndex, Structure,
        look::{
            self, AreaStyle, BoxBorder, Color, HeadingRegion, HorzAlign, LabelPosition, Look,
            RowColBorder, RowParity, Stroke, VertAlign,
        },
        parse_bool,
        value::{self, DatumValue, TemplateValue, ValueFormat, ValueStyle, VariableValue},
    },
    settings,
};

/// A warning decoding a light detail member.
#[derive(Clone, Debug, Display, thiserror::Error)]
pub enum LightWarning {
    /// Unknown encoding {0:?}.
    UnknownEncoding(String),

    /// Unknown "show" value {0}.
    UnknownShow(u8),

    /// Invalid decimal point {0:?}.
    InvalidDecimal(char),

    /// Invalid custom currency definition {0:?}.
    InvalidCustomCurrency(String),

    /// Invalid format type {0}.
    InvalidFormat(u16),

    /// Expected {expected} dimensions along axes, found {actual} dimensions ({n_layers} layers + {n_rows} rows + {n_columns} columns).
    WrongAxisCount {
        /// Expected number of dimensions.
        expected: usize,
        /// Actual number of dimensions.
        actual: usize,
        /// Actual number of layer dimensions.
        n_layers: usize,
        /// Actual number of row dimensions.
        n_rows: usize,
        /// Actual number of column dimensions.
        n_columns: usize,
    },

    /// Invalid dimension index {index} in table with {n} dimensions.
    InvalidDimensionIndex { index: usize, n: usize },

    /// Dimension with index {0} appears twice in table axes.
    DuplicateDimensionIndex(usize),

    /// Dimension {0} has invalid leaf index permutation (so categories may appear out of order).
    InvalidPermutation(usize),

    /// Area {index} has index value {actual} instead of expected {expected}.
    UnexpectedAreaIndex {
        index: usize,
        actual: usize,
        expected: usize,
    },

    /// Area {index} has invalid horizontal alignment {alignment}.
    InvalidHorizontalAlignment { index: usize, alignment: i32 },

    /// Area {index} has invalid vertical alignment {alignment}.
    InvalidVerticalAlignment { index: usize, alignment: i32 },

    /// Area {index} has unknown style value {style:#x}.
    UnknownStyle { index: usize, style: u32 },

    /// Invalid border index {index}.
    InvalidBorderIndex { index: usize },

    /// Border {index} has invalid stroke style {stroke}.
    InvalidStroke { index: usize, stroke: i32 },

    /// Cell index {index} is greater than or equal to the maximum value {n_cells}.
    InvalidCellIndex { index: usize, n_cells: usize },

    /// Data value has invalid horizontal alignment {0}.
    InvalidValueHorizontalAlignment(u32),

    /// Data value has invalid vertical alignment {0}.
    InvalidValueVerticalAlignment(u32),

    /// Current layer {current} is greater than or equal to maximum {maximum}.
    InvalidCurrentLayer { current: usize, maximum: usize },
}

struct Context {
    version: Version,
}

impl Context {
    fn new(version: Version) -> Self {
        Self { version }
    }
}

#[binread]
#[br(little)]
#[derive(Debug)]
pub struct LightTable {
    header: Header,
    #[br(temp, calc(Context::new(header.version)))]
    context: Context,
    #[br(args(&context))]
    titles: Titles,
    #[br(parse_with(parse_vec), args(&context))]
    footnotes: Vec<Footnote>,
    #[br(args(&context))]
    areas: Areas,
    #[br(parse_with(parse_counted))]
    borders: Borders,
    #[br(parse_with(parse_counted))]
    print_settings: PrintSettings,
    #[br(if(header.version == Version::V3), parse_with(parse_counted))]
    table_settings: TableSettings,
    #[br(if(header.version == Version::V1), temp)]
    _ts: Option<Counted<Sponge>>,
    #[br(args(&context))]
    formats: Formats,
    #[br(parse_with(parse_vec), args(&context))]
    dimensions: Vec<Dimension>,
    axes: Axes,
    #[br(parse_with(parse_vec), args(&context))]
    cells: Vec<Cell>,
}

impl LightTable {
    fn decode_look(&self, encoding: &'static Encoding, warn: &mut dyn FnMut(LightWarning)) -> Look {
        Look {
            name: self.table_settings.table_look.decode_optional(encoding),
            hide_empty: self.table_settings.omit_empty,
            row_label_position: if self.table_settings.show_row_labels_in_corner {
                LabelPosition::Corner
            } else {
                LabelPosition::Nested
            },
            heading_widths: enum_map! {
                HeadingRegion::Rows => self.header.min_row_heading_width as isize..=self.header.max_row_heading_width as isize,
                HeadingRegion::Columns => self.header.min_column_heading_width as isize..=self.header.max_column_heading_width as isize,
            },
            footnote_marker_type: if self.table_settings.show_alphabetic_markers {
                FootnoteMarkerType::Alphabetic
            } else {
                FootnoteMarkerType::Numeric
            },
            footnote_marker_position: if self.table_settings.footnote_marker_subscripts {
                FootnoteMarkerPosition::Subscript
            } else {
                FootnoteMarkerPosition::Superscript
            },
            areas: self.areas.decode(encoding, warn),
            borders: self.borders.decode(warn),
            print_all_layers: self.print_settings.alll_layers,
            paginate_layers: self.print_settings.paginate_layers,
            shrink_to_fit: enum_map! {
                Axis2::X => self.print_settings.fit_width,
                Axis2::Y => self.print_settings.fit_length,
            },
            show_continuations: [
                self.print_settings.top_continuation,
                self.print_settings.bottom_continuation,
            ],
            continuation: self
                .print_settings
                .continuation_string
                .decode_optional(encoding),
            n_orphan_lines: self.print_settings.n_orphan_lines,
        }
    }

    pub fn decode(&self, mut warn: &mut dyn FnMut(LightWarning)) -> PivotTable {
        let encoding = self.formats.encoding(warn);

        let n1 = self.formats.n1();
        let n2 = self.formats.n2();
        let n3 = self.formats.n3();
        let n3_inner = n3.and_then(|n3| n3.inner.as_ref());
        let y1 = self.formats.y1();
        let footnotes = self
            .footnotes
            .iter()
            .map(|f| f.decode(encoding, &Footnotes::new(), warn))
            .collect();
        let dimensions = self
            .dimensions
            .iter()
            .enumerate()
            .map(|(dim_index, d)| {
                let mut root = Group::new(d.name.decode(encoding, &footnotes, warn))
                    .with_show_label(!d.hide_dim_label);
                let mut ptod = Vec::new();
                for category in &d.categories {
                    category.decode(encoding, &footnotes, &mut root, &mut ptod, warn);
                }
                let mut dimension =
                    pivot::Dimension::new(root).with_hide_all_labels(d.hide_all_labels);
                if dimension.set_ptod(ptod).is_err() {
                    warn(LightWarning::InvalidPermutation(dim_index));
                };
                dimension
            })
            .collect::<Vec<_>>();
        let structure = match self.axes.decode(dimensions.len()) {
            Ok(axes) => {
                Structure::new(dimensions, axes).expect("Axes::decode pre-validated the structure")
            }
            Err(warning) => {
                warn(warning);
                Structure::from(repeat(Axis3::Y).zip(dimensions))
            }
        };

        let mut remainder = self.table_settings.current_layer;
        let layer = structure
            .axis_dimensions(Axis3::Z)
            .map(|d| {
                if d.len() == 0 {
                    0
                } else {
                    let category = remainder % d.len();
                    remainder /= d.len();
                    category
                }
            })
            .collect::<Vec<_>>();
        if remainder > 0 {
            warn(LightWarning::InvalidCurrentLayer {
                current: self.table_settings.current_layer,
                maximum: structure.axis_extent(Axis3::Z),
            });
        }

        let n_cells = structure.n_cells();
        let cells = self
            .cells
            .iter()
            .filter_map(|cell| {
                if cell.index < n_cells {
                    Some((
                        PrecomputedIndex(cell.index),
                        cell.value.decode(encoding, &footnotes, warn),
                    ))
                } else {
                    warn(LightWarning::InvalidCellIndex {
                        index: cell.index,
                        n_cells,
                    });
                    None
                }
            })
            .collect::<Vec<_>>();
        let pivot_table = PivotTable::new(structure)
            .with_style(PivotTableStyle {
                look: Arc::new(self.decode_look(encoding, warn)),
                rotate_inner_column_labels: self.header.rotate_inner_column_labels,
                rotate_outer_row_labels: self.header.rotate_outer_row_labels,
                show_grid_lines: self.borders.show_grid_lines,
                show_title: n1.map_or(true, |x1| x1.show_title != 10),
                show_caption: n1.map_or(true, |x1| x1.show_caption),
                show_values: n1.map_or(None, |x1| x1.show_values.decode(&mut warn)),
                show_variables: n1.map_or(None, |x1| x1.show_variables.decode(&mut warn)),
                sizing: self.table_settings.sizing.decode(
                    &self.formats.column_widths,
                    n2.map_or(&[], |x2| &x2.row_heights),
                ),
                settings: Settings {
                    epoch: self.formats.y0.epoch(),
                    decimal: self.formats.y0.decimal(warn),
                    leading_zero: if let Some(y1) = y1 {
                        y1.include_leading_zero
                    } else {
                        false
                    },
                    leading_zero_pct: true,
                    ccs: self.formats.custom_currency.decode(encoding, warn),
                },
                grouping: {
                    let grouping = self.formats.y0.grouping;
                    b",.' ".contains(&grouping).then_some(grouping as char)
                },
                small: n3.map_or(0.0, |n3| n3.small),
                weight_format: F40,
            })
            .with_metadata(PivotTableMetadata {
                command_local: y1.map(|y1| y1.command_local.decode(encoding)),
                command_c: y1.map(|y1| y1.command.decode(encoding)),
                language: y1.map(|y1| y1.language.decode(encoding)),
                locale: y1.map(|y1| y1.locale.decode(encoding)),
                dataset: n3_inner.and_then(|strings| strings.dataset.decode_optional(encoding)),
                datafile: n3_inner.and_then(|strings| strings.datafile.decode_optional(encoding)),
                date: n3_inner.and_then(|inner| {
                    if inner.date != 0 {
                        DateTime::from_timestamp(inner.date as i64, 0).map(|dt| dt.naive_utc())
                    } else {
                        None
                    }
                }),
                title: Some(Box::new(
                    self.titles.user_title.decode(encoding, &footnotes, warn),
                )),
                subtype: Some(Box::new(
                    self.titles.subtype.decode(encoding, &footnotes, warn),
                )),
                corner_text: self
                    .titles
                    .corner_text
                    .as_ref()
                    .map(|corner| Box::new(corner.decode(encoding, &footnotes, warn))),
                caption: self
                    .titles
                    .caption
                    .as_ref()
                    .map(|caption| Box::new(caption.decode(encoding, &footnotes, warn))),
                notes: self.table_settings.notes.decode_optional(encoding),
                notes_unexpanded: n3_inner
                    .and_then(|inner| inner.notes_unexpanded.decode_optional(encoding)),
            })
            .with_footnotes(footnotes)
            .with_data(cells)
            .with_layer(&layer);
        pivot_table
    }
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct Header {
    #[br(magic = b"\x01\0")]
    version: Version,
    #[br(parse_with(parse_bool), temp)]
    _x0: bool,
    #[br(parse_with(parse_bool), temp)]
    _x1: bool,
    #[br(parse_with(parse_bool))]
    rotate_inner_column_labels: bool,
    #[br(parse_with(parse_bool))]
    rotate_outer_row_labels: bool,
    #[br(parse_with(parse_bool), temp)]
    _x2: bool,
    #[br(temp)]
    _x3: i32,
    min_column_heading_width: u32,
    max_column_heading_width: u32,
    min_row_heading_width: u32,
    max_row_heading_width: u32,
    #[br(temp)]
    _table_id: i64,
}

#[binread]
#[br(little)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Version {
    #[br(magic = 1u32)]
    V1,
    #[br(magic = 3u32)]
    V3,
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct Titles {
    #[br(args(context))]
    title: Value,
    #[br(temp)]
    _1: Optional<One>,
    #[br(args(context))]
    subtype: Value,
    #[br(temp)]
    _2: Optional<One>,
    #[br(magic = b'1')]
    #[br(args(context))]
    user_title: Value,
    #[br(temp)]
    _3: Optional<One>,
    #[br(parse_with(parse_explicit_optional), args(context))]
    corner_text: Option<Value>,
    #[br(parse_with(parse_explicit_optional), args(context))]
    caption: Option<Value>,
}

#[binread]
#[br(little, magic = 1u8)]
#[derive(Debug)]
struct One;

#[binread]
#[br(little, magic = 0u8)]
#[derive(Debug)]
struct Zero;

#[binrw::parser(reader, endian)]
pub fn parse_explicit_optional<'a, T, A>(args: A, ...) -> BinResult<Option<T>>
where
    T: BinRead<Args<'a> = A>,
{
    let byte = <u8>::read_options(reader, endian, ())?;
    match byte {
        b'1' => Ok(Some(T::read_options(reader, endian, args)?)),
        b'X' => Ok(None),
        _ => Err(BinError::NoVariantMatch {
            pos: reader.stream_position()? - 1,
        }),
    }
}

#[binrw::parser(reader, endian)]
pub(super) fn parse_vec<'a, T, A>(inner: A, ...) -> BinResult<Vec<T>>
where
    T: BinRead<Args<'a> = A>,
    A: Clone,
    T: 'static,
{
    let count = u32::read_options(reader, endian, ())? as usize;
    let mut vec = Vec::with_capacity(count);
    for _ in 0..count {
        vec.push(T::read_options(reader, endian, inner.clone())?);
    }
    Ok(vec)
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct Footnote {
    #[br(args(context))]
    text: Value,
    #[br(parse_with(parse_explicit_optional))]
    #[br(args(context))]
    marker: Option<Value>,
    show: i32,
}

impl Footnote {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> pivot::Footnote {
        pivot::Footnote::new(self.text.decode(encoding, footnotes, warn))
            .with_marker(
                self.marker
                    .as_ref()
                    .map(|m| m.decode(encoding, footnotes, warn)),
            )
            .with_show(self.show > 0)
    }
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct Areas {
    #[br(temp)]
    _zero: Optional<Zero>,
    #[br(args(context))]
    areas: [Area; 8],
}

impl look::Area {
    fn spv_index(&self) -> usize {
        match self {
            look::Area::Title => 0,
            look::Area::Caption => 1,
            look::Area::Footer => 2,
            look::Area::Corner => 3,
            look::Area::Labels(Axis2::X) => 4,
            look::Area::Labels(Axis2::Y) => 5,
            look::Area::Data(_) => 6,
            look::Area::Layers => 7,
        }
    }
}

impl Areas {
    fn decode(
        &self,
        encoding: &'static Encoding,
        warn: &mut dyn FnMut(LightWarning),
    ) -> EnumMap<look::Area, AreaStyle> {
        EnumMap::from_fn(|area| {
            let data_row = match area {
                look::Area::Data(row) => row,
                _ => RowParity::default(),
            };
            let index = area.spv_index();
            self.areas[index].decode(index, encoding, data_row, warn)
        })
    }
}

#[binrw::parser(reader, endian)]
fn parse_color() -> BinResult<Color> {
    let pos = reader.stream_position()?;
    let string = U32String::read_options(reader, endian, ())?;
    let string = string.decode(WINDOWS_1252);
    if string.is_empty() {
        Ok(Color::BLACK)
    } else {
        Color::from_str(&string).map_err(|error| binrw::Error::Custom {
            pos,
            err: Box::new(error),
        })
    }
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct Area {
    #[br(map(|index: u8| index as usize))]
    index: usize,
    #[br(magic = b'1')]
    typeface: U32String,
    size: f32,
    style: u32,
    #[br(parse_with(parse_bool))]
    underline: bool,
    halign: i32,
    valign: i32,
    #[br(parse_with(parse_color))]
    fg: Color,
    #[br(parse_with(parse_color))]
    bg: Color,
    #[br(parse_with(parse_bool))]
    alternate: bool,
    #[br(parse_with(parse_color))]
    alt_fg: Color,
    #[br(parse_with(parse_color))]
    alt_bg: Color,
    #[br(if(context.version == Version::V3))]
    margins: Margins,
}

impl Area {
    fn decode(
        &self,
        index: usize,
        encoding: &'static Encoding,
        data_row: RowParity,
        warn: &mut dyn FnMut(LightWarning),
    ) -> AreaStyle {
        if self.index != index + 1 {
            warn(LightWarning::UnexpectedAreaIndex {
                index,
                actual: self.index,
                expected: index + 1,
            });
        }
        if (self.style & !3) != 0 {
            warn(LightWarning::UnknownStyle {
                index,
                style: self.style,
            });
        }
        AreaStyle {
            cell_style: look::CellStyle {
                horz_align: match self.halign {
                    0 => Some(HorzAlign::Center),
                    2 => Some(HorzAlign::Left),
                    4 => Some(HorzAlign::Right),
                    64173 => None,
                    _ => {
                        warn(LightWarning::InvalidHorizontalAlignment {
                            index,
                            alignment: self.halign,
                        });
                        None
                    }
                },
                vert_align: match self.valign {
                    0 => VertAlign::Middle,
                    3 => VertAlign::Bottom,
                    1 => VertAlign::Top,
                    _ => {
                        warn(LightWarning::InvalidVerticalAlignment {
                            index,
                            alignment: self.valign,
                        });
                        VertAlign::Top
                    }
                },
                margins: enum_map! {
                    Axis2::X => [self.margins.left_margin, self.margins.right_margin],
                    Axis2::Y => [self.margins.top_margin, self.margins.bottom_margin]
                },
            },
            font_style: look::FontStyle {
                bold: (self.style & 1) != 0,
                italic: (self.style & 2) != 0,
                underline: self.underline,
                font: self.typeface.decode(encoding),
                fg: if data_row == RowParity::Odd && self.alternate {
                    self.alt_fg
                } else {
                    self.fg
                },
                bg: if data_row == RowParity::Odd && self.alternate {
                    self.alt_bg
                } else {
                    self.bg
                },
                size: (self.size / 1.33) as i32,
            },
        }
    }
}

#[binread]
#[br(little)]
#[derive(Debug, Default)]
struct Margins {
    left_margin: i32,
    right_margin: i32,
    top_margin: i32,
    bottom_margin: i32,
}

#[binread]
#[br(big)]
#[derive(Debug)]
struct Borders {
    #[br(magic(1u32), parse_with(parse_vec))]
    borders: Vec<Border>,

    #[br(parse_with(parse_bool))]
    show_grid_lines: bool,

    #[br(temp, magic(b"\0\0\0"))]
    _1: (),
}

impl Borders {
    fn decode(
        &self,
        warn: &mut dyn FnMut(LightWarning),
    ) -> EnumMap<look::Border, look::BorderStyle> {
        let mut borders = look::Border::default_borders();
        for border in &self.borders {
            match border.decode_index() {
                Ok(index) => borders[index] = border.decode_style(warn),
                Err(warning) => warn(warning),
            }
        }
        borders
    }
}

#[binread]
#[br(big)]
#[derive(Debug)]
struct Border {
    #[br(map(|index: u32| index as usize))]
    index: usize,
    stroke: i32,
    color: u32,
}

impl Border {
    fn decode_index(&self) -> Result<look::Border, LightWarning> {
        use look::Border::*;

        match self.index {
            0 => Ok(Title),
            1 => Ok(OuterFrame(BoxBorder::Left)),
            2 => Ok(OuterFrame(BoxBorder::Top)),
            3 => Ok(OuterFrame(BoxBorder::Right)),
            4 => Ok(OuterFrame(BoxBorder::Bottom)),
            5 => Ok(InnerFrame(BoxBorder::Left)),
            6 => Ok(InnerFrame(BoxBorder::Top)),
            7 => Ok(InnerFrame(BoxBorder::Right)),
            8 => Ok(InnerFrame(BoxBorder::Bottom)),
            9 => Ok(DataLeft),
            10 => Ok(DataTop),
            11 => Ok(Dimension(RowColBorder(HeadingRegion::Rows, Axis2::X))),
            12 => Ok(Dimension(RowColBorder(HeadingRegion::Rows, Axis2::Y))),
            13 => Ok(Dimension(RowColBorder(HeadingRegion::Columns, Axis2::X))),
            14 => Ok(Dimension(RowColBorder(HeadingRegion::Columns, Axis2::Y))),
            15 => Ok(Category(RowColBorder(HeadingRegion::Rows, Axis2::X))),
            16 => Ok(Category(RowColBorder(HeadingRegion::Rows, Axis2::Y))),
            17 => Ok(Category(RowColBorder(HeadingRegion::Columns, Axis2::X))),
            18 => Ok(Category(RowColBorder(HeadingRegion::Columns, Axis2::Y))),
            _ => Err(LightWarning::InvalidBorderIndex { index: self.index }),
        }
    }

    fn decode_style(&self, warn: &mut dyn FnMut(LightWarning)) -> look::BorderStyle {
        let stroke = match self.stroke {
            0 => Stroke::None,
            1 => Stroke::Solid,
            2 => Stroke::Dashed,
            3 => Stroke::Thick,
            4 => Stroke::Thin,
            6 => Stroke::Double,
            _ => {
                warn(LightWarning::InvalidStroke {
                    index: self.index,
                    stroke: self.stroke,
                });
                Stroke::Solid
            }
        };

        let color = Color::new(
            (self.color >> 16) as u8,
            (self.color >> 8) as u8,
            self.color as u8,
        )
        .with_alpha((self.color >> 24) as u8);

        look::BorderStyle { stroke, color }
    }
}

#[binread]
#[br(big)]
#[derive(Debug)]
struct PrintSettings {
    #[br(magic = b"\0\0\0\x01")]
    #[br(parse_with(parse_bool))]
    alll_layers: bool,
    #[br(parse_with(parse_bool))]
    paginate_layers: bool,
    #[br(parse_with(parse_bool))]
    fit_width: bool,
    #[br(parse_with(parse_bool))]
    fit_length: bool,
    #[br(parse_with(parse_bool))]
    top_continuation: bool,
    #[br(parse_with(parse_bool))]
    bottom_continuation: bool,
    #[br(map(|n: u32| n as usize))]
    n_orphan_lines: usize,
    continuation_string: U32String,
}

#[binread]
#[br(big)]
#[derive(Debug, Default)]
struct TableSettings {
    #[br(temp, magic = 1u32)]
    _x5: i32,
    #[br(map(|n: u32| n as usize))]
    current_layer: usize,
    #[br(parse_with(parse_bool))]
    omit_empty: bool,
    #[br(parse_with(parse_bool))]
    show_row_labels_in_corner: bool,
    #[br(parse_with(parse_bool))]
    show_alphabetic_markers: bool,
    #[br(parse_with(parse_bool))]
    footnote_marker_subscripts: bool,
    #[br(temp)]
    _x6: u8,
    #[br(big, parse_with(parse_counted))]
    sizing: Sizing,
    notes: U32String,
    table_look: U32String,
    #[br(temp)]
    _sponge: Sponge,
}

#[binread]
#[br(big)]
#[derive(Debug, Default)]
struct Sizing {
    #[br(parse_with(parse_vec))]
    row_breaks: Vec<u32>,
    #[br(parse_with(parse_vec))]
    column_breaks: Vec<u32>,
    #[br(parse_with(parse_vec))]
    row_keeps: Vec<(i32, i32)>,
    #[br(parse_with(parse_vec))]
    column_keeps: Vec<(i32, i32)>,
    #[br(parse_with(parse_vec))]
    row_point_keeps: Vec<[i32; 3]>,
    #[br(parse_with(parse_vec))]
    column_point_keeps: Vec<[i32; 3]>,
}

impl Sizing {
    fn decode(
        &self,
        column_widths: &[i32],
        row_heights: &[i32],
    ) -> EnumMap<Axis2, Option<Box<look::Sizing>>> {
        fn decode_axis(
            widths: &[i32],
            breaks: &[u32],
            keeps: &[(i32, i32)],
        ) -> Option<Box<look::Sizing>> {
            if widths.is_empty() && breaks.is_empty() && keeps.is_empty() {
                None
            } else {
                Some(Box::new(look::Sizing {
                    widths: widths.into(),
                    breaks: breaks.into_iter().map(|b| *b as usize).collect(),
                    keeps: keeps
                        .into_iter()
                        .map(|(low, high)| *low as usize..*high as usize)
                        .collect(),
                }))
            }
        }

        enum_map! {
            Axis2::X => decode_axis(column_widths, &self.column_breaks, &self.column_keeps),
            Axis2::Y => decode_axis(row_heights, &self.row_breaks, &self.row_keeps),
        }
    }
}

#[binread]
#[derive(Default)]
pub(super) struct U32String {
    #[br(parse_with(parse_vec))]
    string: Vec<u8>,
}

impl U32String {
    pub(super) fn decode(&self, encoding: &'static Encoding) -> String {
        if let Ok(string) = str::from_utf8(&self.string) {
            string.into()
        } else {
            encoding
                .decode_without_bom_handling(&self.string)
                .0
                .into_owned()
        }
    }
    pub(super) fn decode_optional(&self, encoding: &'static Encoding) -> Option<String> {
        let string = self.decode(encoding);
        if !string.is_empty() {
            Some(string)
        } else {
            None
        }
    }
}

impl Debug for U32String {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.string.iter().map(|c| *c as char).collect::<String>();
        write!(f, "{s:?}")
    }
}

#[derive(Clone, Debug, Default)]
struct Counted<T>(T);

impl<T> Deref for Counted<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> BinRead for Counted<T>
where
    T: BinRead,
{
    type Args<'a> = T::Args<'a>;

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let count = u32::read_options(reader, endian, ())? as u64;
        let start = reader.stream_position()?;
        let end = start + count;
        let mut inner = reader.take_seek(count);
        let result = <T>::read_options(&mut inner, Endian::Little, args)?;
        let pos = inner.stream_position()?;
        if pos != end {
            let consumed = pos - start;
            return Err(binrw::Error::Custom {
                pos,
                err: Box::new(format!(
                    "counted data not exhausted (consumed {consumed} bytes out of {count})"
                )),
            });
        }
        Ok(Self(result))
    }
}

#[binrw::parser(reader, endian)]
fn parse_counted<T, A>(args: A, ...) -> BinResult<T>
where
    for<'a> T: BinRead<Args<'a> = A>,
    A: Clone,
    T: 'static,
{
    Ok(<Counted<T>>::read_options(reader, endian, args)?.0)
}

/// `BinRead` for `Option<T>` always requires the value to be there.  This
/// instead tries to read it and falls back to None if there's no match.
#[derive(Clone, Debug)]
struct Optional<T>(Option<T>);

impl<T> Default for Optional<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> Deref for Optional<T> {
    type Target = Option<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> BinRead for Optional<T>
where
    T: BinRead,
{
    type Args<'a> = T::Args<'a>;

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let start = reader.stream_position()?;
        let result = <T>::read_options(reader, endian, args).ok();
        if result.is_none() {
            reader.seek(std::io::SeekFrom::Start(start))?;
        }
        Ok(Self(result))
    }
}

#[binread]
#[br(little)]
#[br(import(context: &Context))]
#[derive(Debug)]
struct Formats {
    #[br(parse_with(parse_vec))]
    column_widths: Vec<i32>,
    locale: U32String,
    current_layer: i32,
    #[br(temp, parse_with(parse_bool))]
    _x7: bool,
    #[br(temp, parse_with(parse_bool))]
    _x8: bool,
    #[br(temp, parse_with(parse_bool))]
    _x9: bool,
    y0: Y0,
    custom_currency: CustomCurrency,
    #[br(if(context.version == Version::V1))]
    v1: Counted<Optional<N0>>,
    #[br(if(context.version == Version::V3))]
    v3: Option<Counted<FormatsV3>>,
}

impl Formats {
    fn y1(&self) -> Option<&Y1> {
        if let Some(n0) = &**self.v1 {
            Some(&n0.y1)
        } else if let Some(v3) = &self.v3 {
            Some(&v3.n3.y1)
        } else {
            None
        }
    }

    fn n1(&self) -> Option<&N1> {
        self.v3.as_ref().map(|v3| &v3.n1_n2.x1)
    }

    fn n2(&self) -> Option<&N2> {
        self.v3.as_ref().map(|v3| &v3.n1_n2.x2)
    }

    fn n3(&self) -> Option<&N3> {
        self.v3.as_ref().map(|v3| &v3.n3)
    }

    fn charset(&self) -> Option<&U32String> {
        self.y1().map(|y1| &y1.charset)
    }

    fn encoding(&self, warn: &mut dyn FnMut(LightWarning)) -> &'static Encoding {
        if let Some(charset) = self.charset() {
            if let Some(encoding) = Encoding::for_label(&charset.string) {
                return encoding;
            }
            warn(LightWarning::UnknownEncoding(
                String::from_utf8_lossy(&charset.string).into_owned(),
            ));
        }

        if let Ok(locale) = str::from_utf8(&self.locale.string)
            && let Some(dot) = locale.find('.')
            && let Some(encoding) = Encoding::for_label(locale[dot + 1..].as_bytes())
        {
            encoding
        } else {
            WINDOWS_1252
        }
    }
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct FormatsV3 {
    #[br(parse_with(parse_counted))]
    n1_n2: N1N2,
    #[br(parse_with(parse_counted))]
    n3: N3,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct N1N2 {
    x1: N1,
    #[br(parse_with(parse_counted))]
    x2: N2,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct N0 {
    #[br(temp)]
    _bytes: [u8; 14],
    y1: Y1,
    _y2: Y2,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct Y1 {
    command: U32String,
    command_local: U32String,
    language: U32String,
    charset: U32String,
    locale: U32String,
    #[br(temp, parse_with(parse_bool))]
    _x10: bool,
    #[br(parse_with(parse_bool))]
    include_leading_zero: bool,
    #[br(temp, parse_with(parse_bool))]
    _x12: bool,
    #[br(temp, parse_with(parse_bool))]
    _x13: bool,
    _y0: Y0,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct Y2 {
    custom_currency: CustomCurrency,
    missing: u8,
    #[br(temp, parse_with(parse_bool))]
    _x17: bool,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct N1 {
    #[br(temp, parse_with(parse_bool))]
    _x14: bool,
    show_title: u8,
    #[br(temp, parse_with(parse_bool))]
    _x16: bool,
    _lang: u8,
    show_variables: Show,
    show_values: Show,
    #[br(temp)]
    _x18: i32,
    #[br(temp)]
    _x19: i32,
    #[br(temp)]
    _zeros: [u8; 17],
    #[br(temp, parse_with(parse_bool))]
    _x20: bool,
    #[br(parse_with(parse_bool))]
    show_caption: bool,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct Show(u8);

impl Show {
    fn decode<F>(&self, mut warn: F) -> Option<settings::Show>
    where
        F: FnMut(LightWarning),
    {
        match self.0 {
            0 => None,
            1 => Some(settings::Show::Value),
            2 => Some(settings::Show::Label),
            3 => Some(settings::Show::Both),
            other => {
                warn(LightWarning::UnknownShow(other));
                None
            }
        }
    }
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct N2 {
    #[br(parse_with(parse_vec))]
    row_heights: Vec<i32>,
    #[br(parse_with(parse_vec))]
    style_map: Vec<(i64, i16)>,
    #[br(parse_with(parse_vec))]
    styles: Vec<StylePair>,
    #[br(parse_with(parse_counted))]
    tail: Optional<[u8; 8]>,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct N3 {
    #[br(temp, magic = b"\x01\0")]
    _x21: u8,
    #[br(magic = b"\0\0\0")]
    y1: Y1,
    small: f64,
    #[br(magic = 1u8, temp)]
    _one: (),
    inner: Optional<N3Inner>,
    y2: Y2,
    #[br(temp)]
    _tail: Optional<N3Tail>,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct N3Inner {
    dataset: U32String,
    datafile: U32String,
    notes_unexpanded: U32String,
    date: i32,
    #[br(magic = 0u32, temp)]
    _tail: (),
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct N3Tail {
    #[br(temp)]
    _x22: i32,
    #[br(temp, assert(_zero == 0))]
    _zero: i32,
    #[br(temp, assert(_x25.is_none_or(|x25| x25 == 0 || x25 == 1)))]
    _x25: Optional<u8>,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct Y0 {
    epoch: i32,
    decimal: u8,
    grouping: u8,
}

impl Y0 {
    fn epoch(&self) -> Epoch {
        if (1000..=9999).contains(&self.epoch) {
            Epoch(self.epoch)
        } else {
            Epoch::default()
        }
    }

    fn decimal(&self, warn: &mut dyn FnMut(LightWarning)) -> Decimal {
        let c = self.decimal as char;
        match Decimal::try_from(c) {
            Ok(decimal) => decimal,
            Err(_) => {
                if c != '\0' {
                    warn(LightWarning::InvalidDecimal(c));
                }
                Decimal::default()
            }
        }
    }
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct CustomCurrency {
    #[br(parse_with(parse_vec))]
    ccs: Vec<U32String>,
}

impl CustomCurrency {
    fn decode(
        &self,
        encoding: &'static Encoding,
        warn: &mut dyn FnMut(LightWarning),
    ) -> CustomCurrencies {
        let mut ccs = CustomCurrencies::default();
        for (cc, string) in enum_iterator::all().zip(&self.ccs) {
            let string = string.decode(encoding);
            if !string.is_empty() {
                if let Ok(style) = NumberStyle::from_str(&string) {
                    ccs.set(cc, style);
                } else {
                    warn(LightWarning::InvalidCustomCurrency(string));
                }
            }
        }
        ccs
    }
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct ValueNumber {
    #[br(parse_with(parse_explicit_optional), args(context))]
    mods: Option<ValueMods>,
    format: Format,
    x: f64,
}

#[binread]
#[br(little)]
struct Format(u32);

impl Format {
    pub fn decode(&self, warn: &mut dyn FnMut(LightWarning)) -> ValueFormat {
        decode_format(self.0, warn)
    }
}

impl Debug for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut warning = false;
        let format = self.decode(&mut |_| {
            warning = true;
        });
        if warning {
            write!(f, "InvalidFormat({:#x})", self.0)
        } else {
            match format {
                ValueFormat::Other(format) => write!(f, "{format}"),
                ValueFormat::SmallE(format) => write!(f, "SmallE({format})"),
            }
        }
    }
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct ValueVarNumber {
    #[br(parse_with(parse_explicit_optional), args(context))]
    mods: Option<ValueMods>,
    format: Format,
    x: f64,
    var_name: U32String,
    value_label: U32String,
    show: Show,
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct ValueText {
    local: U32String,
    #[br(parse_with(parse_explicit_optional), args(context))]
    mods: Option<ValueMods>,
    id: U32String,
    c: U32String,
    #[br(parse_with(parse_bool))]
    fixed: bool,
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct ValueString {
    #[br(parse_with(parse_explicit_optional), args(context))]
    mods: Option<ValueMods>,
    format: Format,
    value_label: U32String,
    var_name: U32String,
    show: Show,
    s: U32String,
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct ValueVarName {
    #[br(parse_with(parse_explicit_optional), args(context))]
    mods: Option<ValueMods>,
    var_name: U32String,
    var_label: U32String,
    show: Show,
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct ValueFixedText {
    local: U32String,
    #[br(parse_with(parse_explicit_optional), args(context))]
    mods: Option<ValueMods>,
    id: U32String,
    c: U32String,
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct ValueTemplate {
    #[br(parse_with(parse_explicit_optional), args(context))]
    mods: Option<ValueMods>,
    template: U32String,
    #[br(parse_with(parse_vec), args(context))]
    args: Vec<Argument>,
}

#[derive(Debug)]
enum Value {
    Number(ValueNumber),
    VarNumber(ValueVarNumber),
    Text(ValueText),
    String(ValueString),
    VarName(ValueVarName),
    FixedText(ValueFixedText),
    Template(ValueTemplate),
}

impl BinRead for Value {
    type Args<'a> = (&'a Context,);

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let start = reader.stream_position()?;
        let kind = loop {
            let x = <u8>::read_options(reader, endian, ())?;
            if x != 0 {
                break x;
            }
        };
        match kind {
            1 => ValueNumber::read_options(reader, endian, args).map(Self::Number),
            2 => Ok(Self::VarNumber(ValueVarNumber::read_options(
                reader, endian, args,
            )?)),
            3 => Ok(Self::Text(ValueText::read_options(reader, endian, args)?)),
            4 => Ok(Self::String(ValueString::read_options(
                reader, endian, args,
            )?)),
            5 => Ok(Self::VarName(ValueVarName::read_options(
                reader, endian, args,
            )?)),
            6 => Ok(Self::FixedText(ValueFixedText::read_options(
                reader, endian, args,
            )?)),
            b'1' | b'X' => {
                reader.seek(std::io::SeekFrom::Current(-1))?;
                Ok(Self::Template(ValueTemplate::read_options(
                    reader, endian, args,
                )?))
            }
            _ => Err(BinError::NoVariantMatch { pos: start }),
        }
        .map_err(|e| e.with_message(format!("while parsing Value starting at offset {start:#x}")))
    }
}

pub(super) fn decode_format(raw: u32, warn: &mut dyn FnMut(LightWarning)) -> ValueFormat {
    if raw == 0 || raw == 0x10000 || raw == 1 {
        return ValueFormat::Other(F40_2);
    }

    let raw_type = (raw >> 16) as u16;
    let type_ = if raw_type >= 40 {
        Type::F
    } else if let Ok(type_) = Type::try_from(raw_type) {
        type_
    } else {
        warn(LightWarning::InvalidFormat(raw_type));
        Type::F
    };

    let w = ((raw >> 8) & 0xff) as Width;
    let d = raw as Decimals;
    let inner = UncheckedFormat::new(type_, w, d).fix();

    if raw_type >= 40 {
        ValueFormat::SmallE(inner)
    } else {
        ValueFormat::Other(inner)
    }
}

impl ValueNumber {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> value::Value {
        value::Value::new_number((self.x != -f64::MAX).then_some(self.x))
            .with_format(self.format.decode(warn))
            .with_styling(ValueMods::decode_optional(
                &self.mods, encoding, footnotes, warn,
            ))
    }
}

impl ValueVarNumber {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> value::Value {
        value::Value::new_number((self.x != -f64::MAX).then_some(self.x))
            .with_format(self.format.decode(warn))
            .with_styling(ValueMods::decode_optional(
                &self.mods, encoding, footnotes, warn,
            ))
            .with_value_label(self.value_label.decode_optional(encoding))
            .with_variable_name(Some(self.var_name.decode(encoding)))
            .with_show_value_label(self.show.decode(warn))
    }
}

impl ValueText {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> value::Value {
        value::Value::new_general_text(
            self.local.decode(encoding),
            self.c.decode(encoding),
            self.id.decode(encoding),
            !self.fixed,
        )
        .with_styling(ValueMods::decode_optional(
            &self.mods, encoding, footnotes, warn,
        ))
    }
}

impl ValueString {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> value::Value {
        value::Value::new(pivot::value::ValueInner::Datum(DatumValue {
            datum: Datum::new_utf8(self.s.decode(encoding)),
            format: self.format.decode(warn),
            show: self.show.decode(&mut *warn),
            variable: self.var_name.decode_optional(encoding),
            value_label: self.value_label.decode_optional(encoding),
        }))
        .with_styling(ValueMods::decode_optional(
            &self.mods, encoding, footnotes, warn,
        ))
    }
}

impl ValueVarName {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> value::Value {
        value::Value::new(pivot::value::ValueInner::Variable(VariableValue {
            show: self.show.decode(&mut *warn),
            var_name: self.var_name.decode(encoding),
            variable_label: self.var_label.decode_optional(encoding),
        }))
        .with_styling(ValueMods::decode_optional(
            &self.mods, encoding, footnotes, warn,
        ))
    }
}
impl ValueFixedText {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> value::Value {
        value::Value::new_general_text(
            self.local.decode(encoding),
            self.c.decode(encoding),
            self.id.decode(encoding),
            false,
        )
        .with_styling(ValueMods::decode_optional(
            &self.mods, encoding, footnotes, warn,
        ))
    }
}

impl ValueTemplate {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> value::Value {
        value::Value::new(pivot::value::ValueInner::Template(TemplateValue {
            args: self
                .args
                .iter()
                .map(|argument| argument.decode(encoding, footnotes, warn))
                .collect(),
            localized: self.template.decode(encoding),
            id: self
                .mods
                .as_ref()
                .and_then(|mods| mods.template_id(encoding)),
        }))
        .with_styling(ValueMods::decode_optional(
            &self.mods, encoding, footnotes, warn,
        ))
    }
}

impl Value {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> value::Value {
        match self {
            Value::Number(number) => number.decode(encoding, footnotes, warn),
            Value::VarNumber(var_number) => var_number.decode(encoding, footnotes, warn),
            Value::Text(text) => text.decode(encoding, footnotes, warn),
            Value::String(string) => string.decode(encoding, footnotes, warn),
            Value::VarName(var_name) => var_name.decode(encoding, footnotes, warn),
            Value::FixedText(fixed_text) => fixed_text.decode(encoding, footnotes, warn),
            Value::Template(template) => template.decode(encoding, footnotes, warn),
        }
    }
}

#[derive(Debug)]
struct Argument(Vec<Value>);

impl BinRead for Argument {
    type Args<'a> = (&'a Context,);

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        context: Self::Args<'_>,
    ) -> BinResult<Self> {
        let count = u32::read_options(reader, endian, ())? as usize;
        if count == 0 {
            Ok(Self(vec![Value::read_options(reader, endian, context)?]))
        } else {
            let zero = u32::read_options(reader, endian, ())?;
            assert_eq!(zero, 0);
            let values = <Vec<_>>::read_options(
                reader,
                endian,
                VecArgs {
                    count,
                    inner: context,
                },
            )?;
            Ok(Self(values))
        }
    }
}

impl Argument {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> Vec<value::Value> {
        self.0
            .iter()
            .map(|value| value.decode(encoding, footnotes, warn))
            .collect()
    }
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct ValueMods {
    #[br(parse_with(parse_vec))]
    refs: Vec<i16>,
    #[br(parse_with(parse_vec))]
    subscripts: Vec<U32String>,
    #[br(if(context.version == Version::V1))]
    v1: Option<ValueModsV1>,
    #[br(if(context.version == Version::V3), parse_with(parse_counted))]
    v3: ValueModsV3,
}

#[binread]
#[br(little)]
#[derive(Debug, Default)]
struct ValueModsV1 {
    #[br(temp, magic(0u8), assert(_1 == 1 || _1 == 2))]
    _1: i32,
    #[br(temp)]
    _0: Optional<Zero>,
    #[br(temp)]
    _1: Optional<Zero>,
    #[br(temp)]
    _2: i32,
    #[br(temp)]
    _3: Optional<Zero>,
    #[br(temp)]
    _4: Optional<Zero>,
}

#[binread]
#[br(little)]
#[derive(Debug, Default)]
struct ValueModsV3 {
    #[br(parse_with(parse_counted))]
    template_string: Optional<TemplateString>,
    style_pair: StylePair,
}

impl ValueMods {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> ValueStyle {
        let font_style = self
            .v3
            .style_pair
            .font_style
            .as_ref()
            .map(|font_style| look::FontStyle {
                bold: font_style.bold,
                italic: font_style.italic,
                underline: font_style.underline,
                font: font_style.typeface.decode(encoding),
                fg: font_style.fg,
                bg: font_style.bg,
                size: (font_style.size as i32) * 4 / 3,
            });
        let cell_style = self
            .v3
            .style_pair
            .cell_style
            .as_ref()
            .map(|cell_style| look::CellStyle {
                horz_align: match cell_style.halign {
                    0 => Some(HorzAlign::Center),
                    2 => Some(HorzAlign::Left),
                    4 => Some(HorzAlign::Right),
                    6 => Some(HorzAlign::Decimal {
                        offset: cell_style.decimal_offset,
                    }),
                    0xffffffad => None,
                    other => {
                        warn(LightWarning::InvalidValueHorizontalAlignment(other));
                        None
                    }
                },
                vert_align: match cell_style.valign {
                    0 => VertAlign::Middle,
                    3 => VertAlign::Bottom,
                    1 => VertAlign::Top,
                    other => {
                        warn(LightWarning::InvalidValueVerticalAlignment(other));
                        VertAlign::Top
                    }
                },
                margins: enum_map! {
                    Axis2::X => [cell_style.left_margin as i32, cell_style.right_margin as i32],
                    Axis2::Y => [cell_style.top_margin as i32, cell_style.bottom_margin as i32],
                },
            });
        ValueStyle {
            cell_style,
            font_style,
            subscripts: self.subscripts.iter().map(|s| s.decode(encoding)).collect(),
            footnotes: self
                .refs
                .iter()
                .flat_map(|index| footnotes.get(*index as usize))
                .cloned()
                .collect(),
        }
    }
    fn decode_optional(
        mods: &Option<Self>,
        encoding: &'static Encoding,
        footnotes: &pivot::Footnotes,
        warn: &mut dyn FnMut(LightWarning),
    ) -> Option<Box<pivot::value::ValueStyle>> {
        mods.as_ref()
            .map(|mods| Box::new(mods.decode(encoding, footnotes, warn)))
    }
    fn template_id(&self, encoding: &'static Encoding) -> Option<String> {
        self.v3
            .template_string
            .as_ref()
            .and_then(|template_string| template_string.id.as_ref())
            .map(|s| s.decode(encoding))
    }
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct TemplateString {
    #[br(parse_with(parse_counted), temp)]
    _sponge: Sponge,
    #[br(parse_with(parse_explicit_optional))]
    id: Option<U32String>,
}

#[derive(Debug, Default)]
struct Sponge;

impl BinRead for Sponge {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, _endian: Endian, _args: ()) -> BinResult<Self> {
        let mut buf = [0; 32];
        while reader.read(&mut buf)? > 0 {}
        Ok(Self)
    }
}

#[binread]
#[br(little)]
#[derive(Debug, Default)]
struct StylePair {
    #[br(parse_with(parse_explicit_optional))]
    font_style: Option<FontStyle>,
    #[br(parse_with(parse_explicit_optional))]
    cell_style: Option<CellStyle>,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct FontStyle {
    #[br(parse_with(parse_bool))]
    bold: bool,
    #[br(parse_with(parse_bool))]
    italic: bool,
    #[br(parse_with(parse_bool))]
    underline: bool,
    #[br(parse_with(parse_bool))]
    show: bool,
    #[br(parse_with(parse_color))]
    fg: Color,
    #[br(parse_with(parse_color))]
    bg: Color,
    typeface: U32String,
    size: u8,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct CellStyle {
    halign: u32,
    valign: u32,
    decimal_offset: f64,
    left_margin: i16,
    right_margin: i16,
    top_margin: i16,
    bottom_margin: i16,
}

#[binread]
#[br(little)]
#[br(import(context: &Context))]
#[derive(Debug)]
struct Dimension {
    #[br(args(context))]
    name: Value,
    #[br(temp)]
    _x1: u8,
    #[br(temp)]
    _x2: u8,
    #[br(temp)]
    _x3: u32,
    #[br(parse_with(parse_bool))]
    hide_dim_label: bool,
    #[br(parse_with(parse_bool))]
    hide_all_labels: bool,
    #[br(magic(1u8), temp)]
    _dim_index: i32,
    #[br(parse_with(parse_vec), args(context))]
    categories: Vec<Category>,
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct Category {
    #[br(args(context))]
    name: Value,
    #[br(args(context))]
    child: Child,
}

impl Category {
    fn decode(
        &self,
        encoding: &'static Encoding,
        footnotes: &Footnotes,
        group: &mut pivot::Group,
        ptod: &mut Vec<usize>,
        warn: &mut dyn FnMut(LightWarning),
    ) {
        let name = self.name.decode(encoding, footnotes, warn);
        match &self.child {
            Child::Leaf { leaf_index } => {
                if *leaf_index >= 0 {
                    ptod.push(*leaf_index as usize);
                    group.push(pivot::Leaf::new(name));
                }
            }
            Child::Group {
                flatten: true,
                subcategories,
            } => {
                for subcategory in subcategories {
                    subcategory.decode(encoding, footnotes, group, ptod, warn);
                }
            }
            Child::Group {
                flatten: false,
                subcategories,
            } => {
                let mut subgroup = Group::new(name).with_label_shown();
                for subcategory in subcategories {
                    subcategory.decode(encoding, footnotes, &mut subgroup, ptod, warn);
                }
                group.push(subgroup);
            }
        }
    }
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
enum Child {
    Leaf {
        #[br(magic(0u16), parse_with(parse_bool), temp)]
        _x24: bool,
        #[br(magic(b"\x02\0\0\0"))]
        leaf_index: i32,
        #[br(magic(0u32), temp)]
        _zero: (),
    },
    Group {
        #[br(parse_with(parse_bool))]
        flatten: bool,
        #[br(temp, magic(b"\0\x01"))]
        _x23: i32,
        #[br(magic(-1i32), parse_with(parse_vec), args(context))]
        subcategories: Vec<Box<Category>>,
    },
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct Axes {
    #[br(temp)]
    n_layers: u32,
    #[br(temp)]
    n_rows: u32,
    #[br(temp)]
    n_columns: u32,
    #[br(count(n_layers))]
    layers: Vec<u32>,
    #[br(count(n_rows))]
    rows: Vec<u32>,
    #[br(count(n_columns))]
    columns: Vec<u32>,
}

impl Axes {
    fn decode(&self, n: usize) -> Result<EnumMap<Axis3, Axis>, LightWarning> {
        let actual = self.layers.len() + self.rows.len() + self.columns.len();
        if actual != n {
            return Err(LightWarning::WrongAxisCount {
                expected: n,
                actual,
                n_layers: self.layers.len(),
                n_rows: self.rows.len(),
                n_columns: self.columns.len(),
            });
        }

        fn axis_dims(axis: Axis3, dimensions: &[u32]) -> impl Iterator<Item = (Axis3, usize)> {
            repeat(axis).zip(dimensions.iter().copied().map(|x| x.try_into().unwrap()))
        }

        let mut seen = vec![false; n];
        let mut axes: EnumMap<Axis3, Axis> = EnumMap::default();
        for (axis, index) in axis_dims(Axis3::Z, &self.layers)
            .chain(axis_dims(Axis3::Y, &self.rows))
            .chain(axis_dims(Axis3::X, &self.columns))
        {
            match seen.get_mut(index) {
                None => return Err(LightWarning::InvalidDimensionIndex { index, n }),
                Some(true) => return Err(LightWarning::DuplicateDimensionIndex(index)),
                Some(other) => *other = true,
            }
            axes[axis].dimensions.push(index);
        }

        Ok(axes)
    }
}

#[binread]
#[br(little, import(context: &Context))]
#[derive(Debug)]
struct Cell {
    #[br(map(|index: u64| index as usize))]
    index: usize,
    #[br(if(context.version == Version::V1), temp)]
    _zero: Optional<Zero>,
    #[br(args(context))]
    value: Value,
}
