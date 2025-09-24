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
    io::{Cursor, Seek, Write},
    iter::{repeat, repeat_n},
};

use binrw::{BinWrite, Endian};
use chrono::Utc;
use enum_map::EnumMap;
use quick_xml::{
    ElementWriter, Writer as XmlWriter,
    events::{BytesText, attributes::Attribute},
};
use zip::{ZipWriter, write::SimpleFileOptions};

use crate::{
    data::{Datum, EncodedString},
    format::{Format, Type},
    output::{
        Details, Item, Itemlike, Text,
        page::{ChartSize, PageSetup},
        pivot::{
            Axis2, Axis3, Category, Dimension, Footnote, FootnoteMarkerPosition,
            FootnoteMarkerType, Footnotes, Group, Leaf, PivotTable,
            look::{
                Area, AreaStyle, Border, BorderStyle, BoxBorder, CellStyle, Color, FontStyle,
                HeadingRegion, HorzAlign, LabelPosition, RowColBorder, RowParity, Stroke,
                VertAlign,
            },
            value::{Value, ValueFormat, ValueInner, ValueStyle},
        },
    },
    settings::Show,
    spv::{Error, html::Document},
    util::ToSmallString,
};

/// SPSS viewer (SPV) file writer.
pub struct Writer<W>
where
    W: Write + Seek,
{
    writer: ZipWriter<W>,
    next_table_id: u64,
    next_heading_id: u64,
    page_setup: Option<PageSetup>,
}

impl<W> Writer<W>
where
    W: Write + Seek,
{
    /// Creates a new `Writer` to write an SPV file to underlying stream
    /// `writer`.
    pub fn for_writer(writer: W) -> Result<Self, Error> {
        let mut writer = ZipWriter::new(writer);
        writer.start_file("META-INF/MANIFEST.MF", SimpleFileOptions::default())?;
        writer.write_all("allowPivoting=true".as_bytes())?;
        Ok(Self {
            writer,
            next_table_id: 1,
            next_heading_id: 1,
            page_setup: None,
        })
    }

    /// Returns this `Writer` with `page_setup` set up to be written with the
    /// next call to [write](Writer::write).
    ///
    /// Page setup is only written if it is set before the first call to
    /// [write](Writer::write).
    pub fn with_page_setup(mut self, page_setup: PageSetup) -> Self {
        self.set_page_setup(page_setup);
        self
    }

    /// Sets `page_setup` to be written with the next call to
    /// [write](Writer::write).
    ///
    /// Page setup is only written if it is set before the first call to
    /// [write](Writer::write).
    pub fn set_page_setup(&mut self, page_setup: PageSetup) {
        self.page_setup = Some(page_setup);
    }

    /// Closes the underlying file and returns the inner writer and the final
    /// I/O result.
    pub fn close(mut self) -> Result<W, Error> {
        self.writer
            .start_file("META-INF/MANIFEST.MF", SimpleFileOptions::default())?;
        write!(&mut self.writer, "allowPivoting=true")?;
        Ok(self.writer.finish()?)
    }

    fn write_table<X>(
        &mut self,
        item: &Item,
        pivot_table: &PivotTable,
        structure: &mut XmlWriter<X>,
    ) -> Result<(), Error>
    where
        X: Write,
    {
        let table_id = self.next_table_id;
        self.next_table_id += 1;

        let mut content = Vec::new();
        let mut cursor = Cursor::new(&mut content);
        pivot_table.write_le(&mut cursor)?;

        let table_name = format!("{table_id:011}_lightTableData.bin");
        self.writer
            .start_file(&table_name, SimpleFileOptions::default())?;
        self.writer.write_all(&content)?;

        self.container(structure, item, "vtb:table", |element| {
            let subtype = pivot_table.subtype().display(pivot_table).to_string();
            let type_ = match subtype.as_str() {
                "Note" | "Notes" => "note",
                "Warning" | "Warnings" => "warning",
                _ => "table",
            };
            element
                .with_attribute(("tableId", Cow::from(table_id.to_string())))
                .with_attribute((
                    "commandName",
                    pivot_table
                        .metadata
                        .command_local
                        .as_ref()
                        .map_or("", |s| s.as_str()),
                ))
                .with_attribute(("type", type_))
                .with_attribute(("subType", Cow::from(subtype)))
                .write_inner_content(|w| {
                    w.create_element("vtb:tableStructure")
                        .write_inner_content(|w| {
                            w.create_element("vtb:dataPath")
                                .write_text_content(BytesText::new(&table_name))?;
                            Ok(())
                        })?;
                    Ok(())
                })?;
            Ok(())
        })?;
        Ok(())
    }

    fn write_text<X>(
        &mut self,
        item: &Item,
        text: &Text,
        structure: &mut XmlWriter<X>,
    ) -> Result<(), Error>
    where
        X: Write,
    {
        self.container(structure, item, "vtx:text", |w| {
            w.with_attribute(("type", text.type_.as_xml_str()))
                .write_inner_content(|w| {
                    w.create_element("html").write_text_content(BytesText::new(
                        &text.content.display(()).to_string(),
                    ))?;
                    Ok(())
                })?;
            Ok(())
        })
    }

    fn write_item<X>(&mut self, item: &Item, structure: &mut XmlWriter<X>) -> Result<(), Error>
    where
        X: Write,
    {
        match &item.details {
            Details::Graph | Details::Image(_) => todo!(),
            Details::Heading(children) => {
                let mut attributes = Vec::<Attribute>::new();
                if let Some(command_name) = &item.command_name {
                    attributes.push(("commandName", command_name.as_str()).into());
                }
                if !item.show {
                    attributes.push(("visibility", "collapsed").into());
                }
                structure
                    .create_element("heading")
                    .with_attributes(attributes)
                    .write_inner_content(|w| {
                        w.create_element("label")
                            .write_text_content(BytesText::new(&item.label()))?;
                        for child in &children.0 {
                            self.write_item(&child, w).map_err(std::io::Error::other)?;
                        }
                        Ok(())
                    })?;
                Ok(())
            }
            Details::Message(diagnostic) => {
                self.write_text(item, &Text::from(diagnostic.as_ref()), structure)
            }
            Details::Table(pivot_table) => self.write_table(item, pivot_table, structure),
            Details::Text(text) => self.write_text(item, text, structure),
        }
    }

    fn container<X, F>(
        &mut self,
        writer: &mut XmlWriter<X>,
        item: &Item,
        inner_elem: &str,
        closure: F,
    ) -> Result<(), Error>
    where
        X: Write,
        F: FnOnce(ElementWriter<X>) -> Result<(), Error>,
    {
        writer
            .create_element("container")
            .with_attributes(
                item.page_break_before()
                    .then_some(("page-break-before", "always")),
            )
            .with_attribute(("visibility", if item.show { "visible" } else { "hidden" }))
            .write_inner_content(|w| {
                let mut element = w
                    .create_element("label")
                    .write_text_content(BytesText::new(&item.label()))?
                    .create_element(inner_elem);
                if let Some(command_name) = &item.command_name {
                    element = element.with_attribute(("commandName", command_name.as_str()));
                };
                closure(element).map_err(std::io::Error::other)?;
                Ok(())
            })?;
        Ok(())
    }
}

impl BinWrite for PivotTable {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        _args: (),
    ) -> binrw::BinResult<()> {
        // Header.
        (
            1u8,
            0u8,
            3u32,           // version
            SpvBool(true),  // x0
            SpvBool(false), // x1
            SpvBool(self.style.rotate_inner_column_labels),
            SpvBool(self.style.rotate_outer_row_labels),
            SpvBool(true), // x2
            0x15u32,       // x3
            *self.style.look.heading_widths[HeadingRegion::Columns].start() as i32,
            *self.style.look.heading_widths[HeadingRegion::Columns].end() as i32,
            *self.style.look.heading_widths[HeadingRegion::Rows].start() as i32,
            *self.style.look.heading_widths[HeadingRegion::Rows].end() as i32,
            0u64,
        )
            .write_le(writer)?;

        // Titles.
        (
            self.title(),
            self.subtype(),
            Optional(Some(self.title())),
            Optional(self.metadata.corner_text.as_ref()),
            Optional(self.metadata.caption.as_ref()),
        )
            .write_le(writer)?;

        // Footnotes.
        self.footnotes.write_le(writer)?;

        // Areas.
        static SPV_AREAS: [Area; 8] = [
            Area::Title,
            Area::Caption,
            Area::Footer,
            Area::Corner,
            Area::Labels(Axis2::X),
            Area::Labels(Axis2::Y),
            Area::Data(RowParity::Even),
            Area::Layers,
        ];
        for (index, area) in SPV_AREAS.into_iter().enumerate() {
            let odd_data_style = if let Area::Data(_) = area {
                Some(&self.style.look.areas[Area::Data(RowParity::Odd)])
            } else {
                None
            };
            self.style.look.areas[area].write_le_args(writer, (index, odd_data_style))?;
        }

        // Borders.
        static SPV_BORDERS: [Border; 19] = [
            Border::Title,
            Border::OuterFrame(BoxBorder::Left),
            Border::OuterFrame(BoxBorder::Top),
            Border::OuterFrame(BoxBorder::Right),
            Border::OuterFrame(BoxBorder::Bottom),
            Border::InnerFrame(BoxBorder::Left),
            Border::InnerFrame(BoxBorder::Top),
            Border::InnerFrame(BoxBorder::Right),
            Border::InnerFrame(BoxBorder::Bottom),
            Border::DataLeft,
            Border::DataTop,
            Border::Dimension(RowColBorder(HeadingRegion::Rows, Axis2::X)),
            Border::Dimension(RowColBorder(HeadingRegion::Rows, Axis2::Y)),
            Border::Dimension(RowColBorder(HeadingRegion::Columns, Axis2::X)),
            Border::Dimension(RowColBorder(HeadingRegion::Columns, Axis2::Y)),
            Border::Category(RowColBorder(HeadingRegion::Rows, Axis2::X)),
            Border::Category(RowColBorder(HeadingRegion::Rows, Axis2::Y)),
            Border::Category(RowColBorder(HeadingRegion::Columns, Axis2::X)),
            Border::Category(RowColBorder(HeadingRegion::Columns, Axis2::Y)),
        ];
        let borders_start = Count::new(writer)?;
        (1, SPV_BORDERS.len() as u32).write_be(writer)?;
        for (index, border) in SPV_BORDERS.into_iter().enumerate() {
            self.style.look.borders[border].write_be_args(writer, index)?;
        }
        (SpvBool(self.style.show_grid_lines), 0u8, 0u16).write_le(writer)?;
        borders_start.finish_le32(writer)?;

        // Print Settings.
        Counted::new((
            1u32,
            SpvBool(self.style.look.print_all_layers),
            SpvBool(self.style.look.paginate_layers),
            SpvBool(self.style.look.shrink_to_fit[Axis2::X]),
            SpvBool(self.style.look.shrink_to_fit[Axis2::Y]),
            SpvBool(self.style.look.show_continuations[0]),
            SpvBool(self.style.look.show_continuations[1]),
            self.style.look.n_orphan_lines as u32,
            SpvString(
                self.style
                    .look
                    .continuation
                    .as_ref()
                    .map_or("", |s| s.as_str()),
            ),
        ))
        .with_endian(Endian::Little)
        .write_be(writer)?;

        // Table Settings.
        Counted::new((
            1u32,
            4u32,
            self.spv_layer() as u32,
            SpvBool(self.style.look.hide_empty),
            SpvBool(self.style.look.row_label_position == LabelPosition::Corner),
            SpvBool(self.style.look.footnote_marker_type == FootnoteMarkerType::Alphabetic),
            SpvBool(
                self.style.look.footnote_marker_position == FootnoteMarkerPosition::Superscript,
            ),
            0u8,
            Counted::new((
                0u32, // n-row-breaks
                0u32, // n-column-breaks
                0u32, // n-row-keeps
                0u32, // n-column-keeps
                0u32, // n-row-point-keeps
                0u32, // n-column-point-keeps
            )),
            SpvString::optional(&self.metadata.notes),
            SpvString::optional(&self.style.look.name),
            Zeros(82),
        ))
        .with_endian(Endian::Little)
        .write_be(writer)?;

        fn y0(pivot_table: &PivotTable) -> impl for<'a> BinWrite<Args<'a> = ()> {
            (
                pivot_table.style.settings.epoch.0 as u32,
                u8::from(pivot_table.style.settings.decimal),
                b',',
            )
        }

        fn custom_currency(pivot_table: &PivotTable) -> impl for<'a> BinWrite<Args<'a> = ()> {
            (
                5,
                EnumMap::from_fn(|cc| {
                    SpvString(
                        pivot_table
                            .style
                            .settings
                            .number_style(Type::CC(cc))
                            .to_string(),
                    )
                })
                .into_array(),
            )
        }

        fn x1(pivot_table: &PivotTable) -> impl for<'a> BinWrite<Args<'a> = ()> {
            (
                0u8, // x14
                if pivot_table.style.show_title {
                    1u8
                } else {
                    10u8
                },
                0u8, // x16
                0u8, // lang
                Show::as_spv(&pivot_table.style.show_variables),
                Show::as_spv(&pivot_table.style.show_values),
                -1i32, // x18
                -1i32, // x19
                Zeros(17),
                SpvBool(false), // x20
                SpvBool(pivot_table.style.show_caption),
            )
        }

        fn x2() -> impl for<'a> BinWrite<Args<'a> = ()> {
            Counted::new((
                0u32, // n-row-heights
                0u32, // n-style-maps
                0u32, // n-styles,
                0u32,
            ))
        }

        fn y1(pivot_table: &PivotTable) -> impl for<'a> BinWrite<Args<'a> = ()> + use<'_> {
            (
                SpvString::optional(&pivot_table.metadata.command_c),
                SpvString::optional(&pivot_table.metadata.command_local),
                SpvString::optional(&pivot_table.metadata.language),
                SpvString("UTF-8"),
                SpvString::optional(&pivot_table.metadata.locale),
                SpvBool(false), // x10
                SpvBool(pivot_table.style.settings.leading_zero),
                SpvBool(true), // x12
                SpvBool(true), // x13
                y0(pivot_table),
            )
        }

        fn y2(pivot_table: &PivotTable) -> impl for<'a> BinWrite<Args<'a> = ()> {
            (custom_currency(pivot_table), b'.', SpvBool(false))
        }

        fn x3(pivot_table: &PivotTable) -> impl for<'a> BinWrite<Args<'a> = ()> + use<'_> {
            Counted::new((
                1u8,
                0u8,
                4u8, // x21
                0u8,
                0u8,
                0u8,
                y1(pivot_table),
                pivot_table.style.small,
                1u8,
                SpvString::optional(&pivot_table.metadata.dataset),
                SpvString::optional(&pivot_table.metadata.datafile),
                0u32,
                pivot_table
                    .metadata
                    .date
                    .map_or(0i64, |date| date.and_utc().timestamp()),
                y2(pivot_table),
            ))
        }

        // Formats.
        (
            0u32,
            SpvString("en_US.ISO_8859-1:1987"),
            0u32,           // XXX current_layer
            SpvBool(false), // x7
            SpvBool(false), // x8
            SpvBool(false), // x9
            y0(self),
            custom_currency(self),
            Counted::new((Counted::new((x1(self), x2())), x3(self))),
        )
            .write_le(writer)?;

        // Dimensions.
        (self.dimensions().len() as u32).write_le(writer)?;

        let x2 = repeat_n(2, self.axes()[Axis3::Z].dimensions.len())
            .chain(repeat_n(0, self.axes()[Axis3::Y].dimensions.len()))
            .chain(repeat(1));
        for ((index, dimension), x2) in self.dimensions().iter().enumerate().zip(x2) {
            dimension.write_options(writer, endian, (index, x2))?;
        }

        // Axes.
        for axis in [Axis3::Z, Axis3::Y, Axis3::X] {
            (self.axes()[axis].dimensions.len() as u32).write_le(writer)?;
        }
        for axis in [Axis3::Z, Axis3::Y, Axis3::X] {
            for index in self.axes()[axis].dimensions.iter().copied() {
                (index as u32).write_le(writer)?;
            }
        }

        // Cells.
        (self.cells().len() as u32).write_le(writer)?;
        for (index, value) in self.cells() {
            (*index as u64, value).write_le(writer)?;
        }

        Ok(())
    }
}

impl PivotTable {
    fn spv_layer(&self) -> usize {
        let mut layer = 0;
        for (dimension, layer_value) in self
            .axis_dimensions(Axis3::Z)
            .zip(self.layer().iter().copied())
            .rev()
        {
            layer = layer * dimension.len() + layer_value;
        }
        layer
    }
}

impl<W> Writer<W>
where
    W: Write + Seek,
{
    /// Writes `item` to the SPV file.
    pub fn write(&mut self, item: &Item) -> Result<(), Error> {
        let mut headings = XmlWriter::new(Cursor::new(Vec::new()));
        let element = headings
            .create_element("heading")
            .with_attribute((
                "creation-date-time",
                Cow::from(Utc::now().format("%x %x").to_string()),
            ))
            .with_attribute((
                "creator",
                Cow::from(format!(
                    "{} {}",
                    env!("CARGO_PKG_NAME"),
                    env!("CARGO_PKG_VERSION")
                )),
            ))
            .with_attribute(("creator-version", "21"))
            .with_attribute(("xmlns", "http://xml.spss.com/spss/viewer/viewer-tree"))
            .with_attribute((
                "xmlns:vps",
                "http://xml.spss.com/spss/viewer/viewer-pagesetup",
            ))
            .with_attribute(("xmlns:vtx", "http://xml.spss.com/spss/viewer/viewer-text"))
            .with_attribute(("xmlns:vtb", "http://xml.spss.com/spss/viewer/viewer-table"));
        element.write_inner_content(|w| {
            w.create_element("label")
                .write_text_content(BytesText::new("Output"))?;
            if let Some(page_setup) = self.page_setup.take() {
                write_page_setup(&page_setup, w)?;
            }
            self.write_item(item, w).map_err(std::io::Error::other)?;
            Ok(())
        })?;

        let headings = headings.into_inner().into_inner();
        let heading_id = self.next_heading_id;
        self.next_heading_id += 1;
        self.writer.start_file(
            format!(
                "outputViewer{heading_id:010}{}.xml",
                if item.details.is_heading() {
                    "_heading"
                } else {
                    ""
                }
            ),
            SimpleFileOptions::default(),
        )?; // XXX
        self.writer.write_all(&headings)?; // XXX
        Ok(())
    }
}

fn write_page_setup<X>(page_setup: &PageSetup, writer: &mut XmlWriter<X>) -> std::io::Result<()>
where
    X: Write,
{
    fn length(length: paper_sizes::Length) -> Cow<'static, str> {
        Cow::from(length.to_string())
    }

    writer
        .create_element("vps:pageSetup")
        .with_attribute((
            "initial-page-number",
            Cow::from(format!("{}", page_setup.initial_page_number)),
        ))
        .with_attribute((
            "chart-size",
            match page_setup.chart_size {
                ChartSize::AsIs => "as-is",
                ChartSize::FullHeight => "full-height",
                ChartSize::HalfHeight => "half-height",
                ChartSize::QuarterHeight => "quarter-height",
            },
        ))
        .with_attribute(("margin-left", length(page_setup.margins.0[Axis2::X][0])))
        .with_attribute(("margin-right", length(page_setup.margins.0[Axis2::X][1])))
        .with_attribute(("margin-top", length(page_setup.margins.0[Axis2::Y][0])))
        .with_attribute(("margin-bottom", length(page_setup.margins.0[Axis2::Y][1])))
        .with_attribute(("paper-height", length(page_setup.paper.height())))
        .with_attribute(("paper-width", length(page_setup.paper.width())))
        .with_attribute((
            "reference-orientation",
            match page_setup.orientation {
                crate::output::page::Orientation::Portrait => "portrait",
                crate::output::page::Orientation::Landscape => "landscape",
            },
        ))
        .with_attribute(("space-after", length(page_setup.object_spacing)))
        .write_inner_content(|w| {
            write_page_heading(&page_setup.header, "vps:pageHeader", w)?;
            write_page_heading(&page_setup.footer, "vps:pageFooter", w)?;
            Ok(())
        })?;
    Ok(())
}

fn write_page_heading<X>(
    heading: &Document,
    name: &str,
    writer: &mut XmlWriter<X>,
) -> std::io::Result<()>
where
    X: Write,
{
    let element = writer.create_element(name);
    if !heading.is_empty() {
        element.write_inner_content(|w| {
            w.create_element("vps:pageParagraph")
                .write_inner_content(|w| {
                    w.create_element("vtx:text")
                        .with_attribute(("text", "title"))
                        .write_text_content(BytesText::new(&heading.to_html()))?;
                    Ok(())
                })?;
            Ok(())
        })?;
    }
    Ok(())
}

impl BinWrite for Dimension {
    type Args<'a> = (usize, u8);

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        (index, x2): (usize, u8),
    ) -> binrw::BinResult<()> {
        (
            self.root().name(),
            0u8, // x1
            x2,
            2u32, // x3
            SpvBool(!self.root().show_label),
            SpvBool(self.hide_all_labels),
            SpvBool(true),
            index as u32,
            self.root().children().len() as u32,
        )
            .write_options(writer, endian, ())?;

        let mut data_indexes = self.ptod().iter().copied();
        for child in self.root().children() {
            child.write_le(writer, &mut data_indexes)?;
        }
        Ok(())
    }
}

impl Category {
    fn write_le<D, W>(&self, writer: &mut W, data_indexes: &mut D) -> binrw::BinResult<()>
    where
        W: Write + Seek,
        D: Iterator<Item = usize>,
    {
        match self {
            Category::Group(group) => group.write_le(writer, data_indexes),
            Category::Leaf(leaf) => leaf.write_le(writer, data_indexes),
        }
    }
}

impl Leaf {
    fn write_le<D, W>(&self, writer: &mut W, data_indexes: &mut D) -> binrw::BinResult<()>
    where
        W: Write + Seek,
        D: Iterator<Item = usize>,
    {
        (
            self.name(),
            0u8,
            0u8,
            0u8,
            2u32,
            data_indexes
                .next()
                .expect("should have as many data indexes as leaves") as u32,
            0u32,
        )
            .write_le(writer)
    }
}

impl Group {
    fn write_le<D, W>(&self, writer: &mut W, data_indexes: &mut D) -> binrw::BinResult<()>
    where
        W: Write + Seek,
        D: Iterator<Item = usize>,
    {
        (
            self.name(),
            0u8, // merge
            0u8,
            1u8,
            0u32, // x23
            -1i32,
            self.children().len() as u32,
        )
            .write_le(writer)?;

        for child in self.children() {
            child.write_le(writer, data_indexes)?;
        }
        Ok(())
    }
}

impl BinWrite for Footnote {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        (
            &self.content,
            Optional(self.marker.as_ref()),
            if self.show { 1i32 } else { -1 },
        )
            .write_options(writer, endian, args)
    }
}

impl BinWrite for Footnotes {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        (self.len() as u32).write_options(writer, endian, args)?;
        for footnote in self {
            footnote.write_options(writer, endian, args)?;
        }
        Ok(())
    }
}

impl BinWrite for AreaStyle {
    type Args<'a> = (usize, Option<&'a AreaStyle>);

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        (index, odd_data_style): (usize, Option<&AreaStyle>),
    ) -> binrw::BinResult<()> {
        let typeface = if self.font_style.font.is_empty() {
            "SansSerif"
        } else {
            self.font_style.font.as_str()
        };
        (
            (index + 1) as u8,
            0x31u8,
            SpvString(typeface),
            self.font_style.size as f32 * 1.33,
            self.font_style.bold as u32 + 2 * self.font_style.italic as u32,
            SpvBool(self.font_style.underline),
            self.cell_style
                .horz_align
                .map_or(64173, |horz_align| horz_align.as_spv(61453)),
            self.cell_style.vert_align.as_spv(),
            self.font_style.fg,
            self.font_style.bg,
        )
            .write_options(writer, endian, ())?;

        let alt_fg = odd_data_style.map_or(self.font_style.fg, |style| style.font_style.fg);
        let alt_bg = odd_data_style.map_or(self.font_style.bg, |style| style.font_style.bg);
        if self.font_style.fg != alt_fg || self.font_style.bg != alt_bg {
            (SpvBool(true), alt_fg, alt_bg).write_options(writer, endian, ())?;
        } else {
            (SpvBool(false), SpvString(""), SpvString("")).write_options(writer, endian, ())?;
        }

        (
            self.cell_style.margins[Axis2::X][0],
            self.cell_style.margins[Axis2::X][1],
            self.cell_style.margins[Axis2::Y][0],
            self.cell_style.margins[Axis2::Y][1],
        )
            .write_options(writer, endian, ())
    }
}

impl Stroke {
    fn as_spv(&self) -> u32 {
        match self {
            Stroke::None => 0,
            Stroke::Solid => 1,
            Stroke::Dashed => 2,
            Stroke::Thick => 3,
            Stroke::Thin => 4,
            Stroke::Double => 5,
        }
    }
}

impl Color {
    fn as_spv(&self) -> u32 {
        ((self.alpha as u32) << 24)
            | ((self.r as u32) << 16)
            | ((self.g as u32) << 8)
            | (self.b as u32)
    }
}

impl BinWrite for BorderStyle {
    type Args<'a> = usize;

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        _endian: Endian,
        index: usize,
    ) -> binrw::BinResult<()> {
        (index as u32, self.stroke.as_spv(), self.color.as_spv()).write_be(writer)
    }
}

struct SpvBool(bool);
impl BinWrite for SpvBool {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        (self.0 as u8).write_options(writer, endian, args)
    }
}

struct SpvString<T>(T);
impl<'a> SpvString<&'a str> {
    fn optional(s: &'a Option<String>) -> Self {
        Self(s.as_ref().map_or("", |s| s.as_str()))
    }
}
impl<T> BinWrite for SpvString<T>
where
    T: AsRef<str>,
{
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        let s = self.0.as_ref();
        let length = s.len() as u32;
        (length, s.as_bytes()).write_options(writer, endian, args)
    }
}

impl Show {
    fn as_spv(this: &Option<Show>) -> u8 {
        match this {
            None => 0,
            Some(Show::Value) => 1,
            Some(Show::Label) => 2,
            Some(Show::Both) => 3,
        }
    }
}

struct Count(u64);

impl Count {
    fn new<W>(writer: &mut W) -> binrw::BinResult<Self>
    where
        W: Write + Seek,
    {
        0u32.write_le(writer)?;
        Ok(Self(writer.stream_position()?))
    }

    fn finish<W>(self, writer: &mut W, endian: Endian) -> binrw::BinResult<()>
    where
        W: Write + Seek,
    {
        let saved_position = writer.stream_position()?;
        let n_bytes = saved_position - self.0;
        writer.seek(std::io::SeekFrom::Start(self.0 - 4))?;
        (n_bytes as u32).write_options(writer, endian, ())?;
        writer.seek(std::io::SeekFrom::Start(saved_position))?;
        Ok(())
    }

    fn finish_le32<W>(self, writer: &mut W) -> binrw::BinResult<()>
    where
        W: Write + Seek,
    {
        self.finish(writer, Endian::Little)
    }
}

struct Counted<T> {
    inner: T,
    endian: Option<Endian>,
}

impl<T> Counted<T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            endian: None,
        }
    }
    fn with_endian(self, endian: Endian) -> Self {
        Self {
            inner: self.inner,
            endian: Some(endian),
        }
    }
}

impl<T> BinWrite for Counted<T>
where
    T: BinWrite,
    for<'a> T: BinWrite<Args<'a> = ()>,
{
    type Args<'a> = T::Args<'a>;

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        let start = Count::new(writer)?;
        self.inner.write_options(writer, endian, args)?;
        start.finish(writer, self.endian.unwrap_or(endian))
    }
}

pub struct Zeros(pub usize);

impl BinWrite for Zeros {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        _endian: Endian,
        _args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        for _ in 0..self.0 {
            writer.write_all(&[0u8])?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct StylePair<'a> {
    font_style: Option<&'a FontStyle>,
    cell_style: Option<&'a CellStyle>,
}

impl BinWrite for Color {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        SpvString(&self.without_alpha().display_css().to_small_string::<16>())
            .write_options(writer, endian, args)
    }
}

impl BinWrite for FontStyle {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        let typeface = if self.font.is_empty() {
            "SansSerif"
        } else {
            self.font.as_str()
        };
        (
            SpvBool(self.bold),
            SpvBool(self.italic),
            SpvBool(self.underline),
            SpvBool(true),
            self.fg,
            self.bg,
            SpvString(typeface),
            (self.size as f64 * 1.33).ceil() as u8,
        )
            .write_options(writer, endian, args)
    }
}

impl HorzAlign {
    fn as_spv(&self, decimal: u32) -> u32 {
        match self {
            HorzAlign::Right => 4,
            HorzAlign::Left => 2,
            HorzAlign::Center => 0,
            HorzAlign::Decimal { .. } => decimal,
        }
    }

    /// Returns the decimal offset for [HorzAlign::Decimal], or `None` for other
    /// horizontal alignments.
    pub fn decimal_offset(&self) -> Option<f64> {
        match *self {
            HorzAlign::Decimal { offset } => Some(offset),
            _ => None,
        }
    }
}

impl VertAlign {
    fn as_spv(&self) -> u32 {
        match self {
            VertAlign::Top => 1,
            VertAlign::Middle => 0,
            VertAlign::Bottom => 3,
        }
    }
}

impl BinWrite for CellStyle {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        (
            self.horz_align
                .map_or(0xffffffad, |horz_align| horz_align.as_spv(6)),
            self.vert_align.as_spv(),
            self.horz_align
                .map(|horz_align| horz_align.decimal_offset())
                .unwrap_or_default(),
            u16::try_from(self.margins[Axis2::X][0]).unwrap_or_default(),
            u16::try_from(self.margins[Axis2::X][1]).unwrap_or_default(),
            u16::try_from(self.margins[Axis2::Y][0]).unwrap_or_default(),
            u16::try_from(self.margins[Axis2::Y][1]).unwrap_or_default(),
        )
            .write_options(writer, endian, args)
    }
}

impl<'a> BinWrite for StylePair<'a> {
    type Args<'b> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        (
            Optional(self.font_style.as_ref()),
            Optional(self.cell_style.as_ref()),
        )
            .write_options(writer, endian, args)
    }
}

struct Optional<T>(Option<T>);

impl<T> BinWrite for Optional<T>
where
    T: BinWrite,
{
    type Args<'a> = T::Args<'a>;

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        match &self.0 {
            Some(value) => {
                0x31u8.write_le(writer)?;
                value.write_options(writer, endian, args)
            }
            None => 0x58u8.write_le(writer),
        }
    }
}

struct ValueMod<'a> {
    style: &'a Option<Box<ValueStyle>>,
    template: Option<&'a str>,
}

impl<'a> ValueMod<'a> {
    fn new(value: &'a Value) -> Self {
        Self {
            style: &value.styling,
            template: None,
        }
    }
}

impl<'a> Default for ValueMod<'a> {
    fn default() -> Self {
        Self {
            style: &None,
            template: None,
        }
    }
}

impl<'a> BinWrite for ValueMod<'a> {
    type Args<'b> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        if self.style.as_ref().is_some_and(|style| !style.is_empty()) || self.template.is_some() {
            0x31u8.write_options(writer, endian, args)?;
            let default_style = Default::default();
            let style = self.style.as_ref().unwrap_or(&default_style);

            (style.footnotes.len() as u32).write_options(writer, endian, args)?;
            for footnote in &style.footnotes {
                (footnote.index() as u16).write_options(writer, endian, args)?;
            }

            (style.subscripts.len() as u32).write_options(writer, endian, args)?;
            for subscript in &style.subscripts {
                SpvString(subscript.as_str()).write_options(writer, endian, args)?;
            }
            let v3_start = Count::new(writer)?;
            let template_string_start = Count::new(writer)?;
            if let Some(template) = self.template {
                Count::new(writer)?.finish_le32(writer)?;
                (0x31u8, SpvString(template)).write_options(writer, endian, args)?;
            }
            template_string_start.finish_le32(writer)?;
            StylePair {
                font_style: style.font_style.as_ref(),
                cell_style: style.cell_style.as_ref(),
            }
            .write_options(writer, endian, args)?;
            v3_start.finish_le32(writer)
        } else {
            0x58u8.write_options(writer, endian, args)
        }
    }
}

impl BinWrite for ValueFormat {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        let type_ = if let ValueFormat::SmallE(format) = self
            && format.type_() == Type::F
        {
            40
        } else {
            self.inner().type_().into()
        };
        let inner = self.inner();
        (((type_ as u32) << 16) | ((inner.w() as u32) << 8) | (inner.d() as u32))
            .write_options(writer, endian, args)
    }
}

impl BinWrite for Value {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> binrw::BinResult<()> {
        match &self.inner {
            ValueInner::Datum(number_value) => match &number_value.datum {
                Datum::Number(number) => {
                    if number_value.variable.is_some() || number_value.value_label.is_some() {
                        (
                            2u8,
                            ValueMod::new(self),
                            number_value.format,
                            number.unwrap_or(f64::MIN),
                            SpvString::optional(&number_value.variable),
                            SpvString::optional(&number_value.value_label),
                            Show::as_spv(&number_value.show),
                        )
                            .write_options(writer, endian, args)?;
                    } else {
                        (
                            1u8,
                            ValueMod::new(self),
                            number_value.format,
                            number.unwrap_or(f64::MIN),
                        )
                            .write_options(writer, endian, args)?;
                    }
                }
                Datum::String(s) => {
                    let hex;
                    let utf8;
                    let (s, format) = if number_value.format.inner().type_() == Type::AHex {
                        hex = s.inner.to_hex();
                        (
                            hex.as_str(),
                            Format::new(Type::AHex, hex.len().min(65534) as u16, 0).unwrap(),
                        )
                    } else {
                        utf8 = s.as_str();
                        (
                            utf8.as_ref(),
                            Format::new(Type::A, utf8.len().min(32767) as u16, 0).unwrap(),
                        )
                    };
                    (
                        4u8,
                        ValueMod::new(self),
                        ValueFormat::Other(format),
                        SpvString::optional(&number_value.value_label),
                        SpvString::optional(&number_value.variable),
                        Show::as_spv(&number_value.show),
                        SpvString(s),
                    )
                        .write_options(writer, endian, args)?;
                }
            },
            ValueInner::Variable(variable) => {
                (
                    5u8,
                    ValueMod::new(self),
                    SpvString(&variable.var_name),
                    SpvString::optional(&variable.variable_label),
                    Show::as_spv(&variable.show),
                )
                    .write_options(writer, endian, args)?;
            }
            ValueInner::Markup(markup) => {
                let text = markup.to_string();
                (
                    3u8,
                    SpvString(&text), // XXX
                    ValueMod::new(self),
                    SpvString(&text),
                    SpvString(&text),
                    SpvBool(true),
                )
                    .write_options(writer, endian, args)?;
            }
            ValueInner::Text(text) => {
                (
                    3u8,
                    SpvString(&text.localized),
                    ValueMod::new(self),
                    SpvString(text.id()),
                    SpvString(text.c()),
                    SpvBool(true),
                )
                    .write_options(writer, endian, args)?;
            }
            ValueInner::Template(template) => {
                (
                    0u8,
                    ValueMod::new(self),
                    SpvString(&template.localized),
                    template.args.len() as u32,
                )
                    .write_options(writer, endian, args)?;
                for arg in &template.args {
                    if arg.len() > 1 {
                        (arg.len() as u32, 0u32).write_options(writer, endian, args)?;
                        for (index, value) in arg.iter().enumerate() {
                            if index > 0 {
                                0u32.write_le(writer)?;
                            }
                            value.write_options(writer, endian, args)?;
                        }
                    } else {
                        (0u32, arg).write_options(writer, endian, args)?;
                    }
                }
            }
            ValueInner::Empty => {
                (
                    3u8,
                    SpvString(""),
                    ValueMod::default(),
                    SpvString(""),
                    SpvString(""),
                    SpvBool(true),
                )
                    .write_options(writer, endian, args)?;
            }
        }
        Ok(())
    }
}
