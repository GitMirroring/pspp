#![allow(missing_docs)]
use std::{
    borrow::Cow,
    io::{BufRead, BufReader, Cursor, Read, Seek},
    mem::take,
    sync::Arc,
};

use anyhow::Context;
use binrw::{BinRead, error::ContextExt};
use cairo::ImageSurface;
use zip::ZipArchive;

use crate::{
    output::{
        Details, Item, ItemKind, ItemRefIterator, Itemlike, SpvInfo, SpvMembers, Text,
        page::PageSetup,
        pivot::{
            Length,
            look::{HorzAlign, Look},
        },
    },
    spv::{
        Error,
        legacy_bin::LegacyBin,
        read::{TableType, Warning, WarningDetails, legacy_xml::Visualization, light::LightTable},
    },
};

/// Part of the outline of an SPV file.
#[derive(Clone, Debug)]
pub struct StructureMember {
    /// The [PageSetup] in the file, if any.
    pub page_setup: Option<PageSetup>,

    /// The contents.
    pub items: Vec<OutlineItem>,
}

impl StructureMember {
    pub fn into_parts(self) -> (Option<PageSetup>, Vec<OutlineItem>) {
        (self.page_setup, self.items)
    }

    pub fn read<R>(
        reader: R,
        member_name: &str,
        warn: &mut dyn FnMut(Warning),
    ) -> Result<Self, Error>
    where
        R: BufRead,
    {
        let mut heading: raw::Heading =
            serde_path_to_error::deserialize(&mut quick_xml::de::Deserializer::from_reader(reader))
                .map_err(|error| Error::DeserializeError {
                    member_name: member_name.into(),
                    error,
                })?;
        Ok(Self {
            page_setup: heading
                .page_setup
                .take()
                .map(|page_setup| page_setup.decode(warn, member_name)),
            items: heading.decode(member_name, warn),
        })
    }
}

#[derive(Clone, Debug)]
pub struct OutlineHeading {
    structure_member: String,
    expand: bool,
    label: String,
    children: Vec<Arc<OutlineItem>>,
    command_name: Option<String>,
}

impl OutlineHeading {
    pub fn spv_info(&self) -> SpvInfo {
        SpvInfo::new(&self.structure_member)
    }
}

#[derive(Clone, Debug)]
pub enum OutlineItem {
    Heading(OutlineHeading),
    Container(Container),
}

impl OutlineItem {
    pub fn read_item<R, F>(self, archive: &mut ZipArchive<R>, warn: &mut F) -> Item
    where
        R: Read + Seek,
        F: FnMut(Warning),
    {
        match self {
            OutlineItem::Container(container) => {
                let mut spv_info = container.spv_info().clone();
                let result = match container.content {
                    Content::Table(table) => table.decode(archive, &mut *warn),
                    Content::Graph(_) => Err(Error::GraphTodo),
                    Content::Text(container_text) => Ok(container_text.into_item()),
                    Content::Image(image) => image.decode(archive),
                    Content::Model => Err(Error::ModelTodo),
                    Content::Tree => Err(Error::TreeTodo),
                };
                spv_info.error = result.is_err();
                result
                    .unwrap_or_else(|error| {
                        Text::new_log(error.to_string())
                            .into_item()
                            .with_label("Error")
                    })
                    .with_show(container.show)
                    .with_command_name(Some(container.command_name))
                    .with_label(container.label)
            }
            OutlineItem::Heading(mut heading) => {
                let expand = heading.expand;
                let label = take(&mut heading.label);
                let command_name = take(&mut heading.command_name);
                let spv_info = heading.spv_info();
                heading
                    .children
                    .into_iter()
                    .map(|child| Arc::unwrap_or_clone(child).read_item(archive, &mut *warn))
                    .collect::<Item>()
                    .with_show(expand)
                    .with_label(label)
                    .with_command_name(command_name)
                    .with_spv_info(spv_info)
            }
        }
    }
}

impl Itemlike for OutlineItem {
    fn label(&self) -> Cow<'_, str> {
        match self {
            OutlineItem::Heading(outline_heading) => Cow::from(outline_heading.label.as_str()),
            OutlineItem::Container(container) => Cow::from(container.label.as_str()),
        }
    }

    fn command_name(&self) -> Option<&str> {
        match self {
            OutlineItem::Heading(outline_heading) => outline_heading.command_name.as_deref(),
            OutlineItem::Container(container) => Some(&container.command_name),
        }
    }

    fn subtype(&self) -> Option<String> {
        match self {
            OutlineItem::Heading(_) => None,
            OutlineItem::Container(container) => container.content.subtype().map(|s| s.into()),
        }
    }

    fn is_shown(&self) -> Option<bool> {
        match self {
            OutlineItem::Heading(_) => None,
            OutlineItem::Container(container) => Some(container.show),
        }
    }

    fn is_expanded(&self) -> Option<bool> {
        match self {
            OutlineItem::Heading(outline_heading) => Some(outline_heading.expand),
            OutlineItem::Container(_) => None,
        }
    }

    fn page_break_before(&self) -> bool {
        match self {
            OutlineItem::Heading(_) => false,
            OutlineItem::Container(container) => container.page_break_before,
        }
    }

    fn spv_info(&self) -> Option<Cow<'_, SpvInfo>> {
        let spv_info = match self {
            OutlineItem::Heading(outline_heading) => outline_heading.spv_info(),
            OutlineItem::Container(container) => container.spv_info(),
        };
        Some(Cow::Owned(spv_info))
    }

    fn iter_in_order(&self) -> ItemRefIterator<'_, Self>
    where
        Self: Sized,
    {
        ItemRefIterator::new(self)
    }

    fn children(&self) -> &[Arc<Self>] {
        match self {
            OutlineItem::Heading(outline_heading) => &outline_heading.children,
            OutlineItem::Container(_) => &[],
        }
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Arc<Self>>> {
        match self {
            OutlineItem::Heading(outline_heading) => Some(&mut outline_heading.children),
            OutlineItem::Container(_) => None,
        }
    }

    fn kind(&self) -> ItemKind {
        match self {
            OutlineItem::Heading(_) => ItemKind::Heading,
            OutlineItem::Container(container) => container.content.kind(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Container {
    structure_member: String,
    show: bool,
    page_break_before: bool,
    text_align: Option<HorzAlign>,
    command_name: String,
    width: Option<Length>,
    label: String,
    content: Content,
}

impl Container {
    pub fn spv_info(&self) -> SpvInfo {
        SpvInfo::new(&self.structure_member).with_members(self.content.members())
    }
}

#[derive(Clone, Debug)]
pub enum Content {
    Text(Text),
    Table(Table),
    Image(Image),
    Graph(Graph),
    Tree,
    Model,
}

impl Content {
    fn kind(&self) -> ItemKind {
        match self {
            Content::Text(_) => ItemKind::Text,
            Content::Table(_) => ItemKind::Table,
            Content::Image(_) => ItemKind::Image,
            Content::Graph(_) => ItemKind::Graph,
            Content::Tree => ItemKind::Tree,
            Content::Model => ItemKind::Model,
        }
    }

    fn subtype(&self) -> Option<&str> {
        match self {
            Content::Table(table) => Some(&table.subtype),
            _ => None,
        }
    }

    fn members(&self) -> Option<SpvMembers> {
        match self {
            Content::Text(_text) => None,
            Content::Table(table) => Some(table.members()),
            Content::Image(image) => Some(image.members()),
            Content::Graph(graph) => Some(graph.members()),
            Content::Tree => None,
            Content::Model => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Table {
    subtype: String,
    table_type: TableType,
    look: Option<Box<Look>>,
    bin_member: String,
    xml_member: Option<String>,
}

impl Table {
    fn members(&self) -> SpvMembers {
        if let Some(xml_member) = &self.xml_member {
            SpvMembers::LegacyTable {
                xml: xml_member.clone(),
                binary: self.bin_member.clone(),
            }
        } else {
            SpvMembers::LightTable(self.bin_member.clone())
        }
    }

    fn decode<R, F>(self, archive: &mut ZipArchive<R>, mut warn: F) -> Result<Item, Error>
    where
        R: Read + Seek,
        F: FnMut(Warning),
    {
        if let Some(xml_member_name) = &self.xml_member {
            let bin_member_name = &self.bin_member;
            let mut bin_member = archive.by_name(bin_member_name)?;
            let mut bin_data = Vec::with_capacity(bin_member.size() as usize);
            bin_member.read_to_end(&mut bin_data)?;
            let mut cursor = Cursor::new(bin_data);
            let legacy_bin = LegacyBin::read(&mut cursor).map_err(|e| {
                e.with_message(format!(
                    "While parsing {bin_member_name:?} as legacy binary SPV member"
                ))
            })?;
            let data = legacy_bin.decode(&mut |w| {
                warn(Warning {
                    member: bin_member_name.clone(),
                    details: WarningDetails::LegacyBinWarning(w),
                })
            });
            drop(bin_member);

            let member = BufReader::new(archive.by_name(&xml_member_name)?);
            let visualization: Visualization = match serde_path_to_error::deserialize(
                &mut quick_xml::de::Deserializer::from_reader(member),
            )
            .with_context(|| format!("Failed to parse {xml_member_name}"))
            {
                Ok(result) => result,
                Err(error) => panic!("{error:?}"),
            };
            let pivot_table =
                visualization.decode(data, *self.look.unwrap_or_default(), &mut |w| {
                    warn(Warning {
                        member: xml_member_name.clone(),
                        details: WarningDetails::LegacyXmlWarning(w),
                    })
                })?;

            Ok(pivot_table.into_item())
        } else {
            let member_name = self.bin_member.clone();
            let mut light = archive.by_name(&member_name)?;
            let mut data = Vec::with_capacity(light.size() as usize);
            light.read_to_end(&mut data)?;
            let mut cursor = Cursor::new(data);

            let table = LightTable::read(&mut cursor).map_err(|e| {
                e.with_message(format!(
                    "While parsing {member_name:?} as light binary SPV member"
                ))
            })?;
            let pivot_table = table.decode(&mut |warning| {
                warn(Warning {
                    member: member_name.clone(),
                    details: WarningDetails::LightWarning(warning),
                })
            });
            Ok(pivot_table.into_item())
        }
    }
}

#[derive(Clone, Debug)]
pub struct Image {
    member: String,
}

impl Image {
    fn members(&self) -> SpvMembers {
        SpvMembers::Image(self.member.clone())
    }
    fn decode<R>(self, archive: &mut ZipArchive<R>) -> Result<Item, Error>
    where
        R: Read + Seek,
    {
        let mut png = archive.by_name(&self.member)?;
        let image = ImageSurface::create_from_png(&mut png)?;
        Ok(Details::Image(image).into_item())
    }
}

#[derive(Clone, Debug)]
pub struct Graph {
    xml_member: String,
    data_member: Option<String>,
    csv_member: Option<String>,
}

impl Graph {
    fn members(&self) -> SpvMembers {
        SpvMembers::Graph {
            data: self.data_member.clone(),
            xml: self.xml_member.clone(),
            csv: self.csv_member.clone(),
        }
    }
}

mod raw {
    use std::{mem::take, sync::Arc};

    use paper_sizes::PaperSize;
    use serde::Deserialize;

    use crate::{
        output::{
            Text,
            page::{self, Orientation},
            pivot::{
                Axis2, Length, TableProperties,
                look::{HorzAlign, Look},
                value::Value,
            },
        },
        spv::{
            html::{self, Document},
            read::{
                TableType, Warning, WarningDetails,
                structure::{Content, OutlineItem},
            },
        },
    };

    use super::OutlineHeading;

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Heading {
        #[serde(rename = "@visibility")]
        visibility: Option<String>,
        #[serde(rename = "@commandName")]
        command_name: Option<String>,
        label: Label,
        pub page_setup: Option<PageSetup>,

        #[serde(rename = "$value")]
        #[serde(default)]
        children: Vec<HeadingContent>,
    }

    impl Heading {
        pub fn decode(
            self,
            structure_member: &str,
            warn: &mut dyn FnMut(Warning),
        ) -> Vec<OutlineItem> {
            let mut items = Vec::new();
            for child in self.children {
                match child {
                    HeadingContent::Container(container) => {
                        let (content, command_name) = match container.content {
                            ContainerContent::Table(table) => (
                                Content::Table(super::Table {
                                    subtype: table.subtype,
                                    table_type: table.table_type,
                                    look: table.table_properties.map(|table_properties| {
                                        Box::new(Look::from(*table_properties))
                                    }),
                                    bin_member: table.table_structure.bin_member,
                                    xml_member: table.table_structure.xml_member,
                                }),
                                table.command_name,
                            ),
                            ContainerContent::Graph(graph) => (
                                Content::Graph(super::Graph {
                                    xml_member: graph.xml_member,
                                    data_member: graph.data_member,
                                    csv_member: graph.csv_member,
                                }),
                                graph.command_name,
                            ),
                            ContainerContent::Text(text) => (
                                Content::Text(Text::new(
                                    match text.text_type {
                                        TextType::Title => crate::output::TextType::Title,
                                        TextType::Log | TextType::Text => {
                                            crate::output::TextType::Log
                                        }
                                        TextType::PageTitle => crate::output::TextType::PageTitle,
                                    },
                                    text.decode(),
                                )),
                                text.command_name.unwrap_or_default(),
                            ),
                            ContainerContent::Image(image) => (
                                Content::Image(super::Image {
                                    member: image.data_path,
                                }),
                                image.command_name.unwrap_or_default(),
                            ),
                            ContainerContent::Object(object) => (
                                Content::Image(super::Image { member: object.uri }),
                                object.command_name.unwrap_or_default(),
                            ),
                            ContainerContent::Model(model) => (Content::Model, model.command_name),
                            ContainerContent::Tree(tree) => (Content::Tree, tree.command_name),
                        };
                        items.push(OutlineItem::Container(super::Container {
                            structure_member: structure_member.into(),
                            show: container.visibility != Visibility::Hidden,
                            page_break_before: container.page_break_before
                                == PageBreakBefore::Always,
                            text_align: container.text_align.map(HorzAlign::from),
                            width: container.width,
                            label: container.label.text,
                            command_name,
                            content,
                        }));
                    }
                    HeadingContent::Heading(mut heading) => {
                        items.push(OutlineItem::Heading(OutlineHeading {
                            structure_member: structure_member.into(),
                            expand: !heading.visibility.is_some(),
                            label: take(&mut heading.label.text),
                            command_name: heading.command_name.take(),
                            children: heading
                                .decode(structure_member, warn)
                                .into_iter()
                                .map(Arc::new)
                                .collect(),
                        }));
                    }
                }
            }
            items
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PageSetup {
        #[serde(rename = "@initial-page-number")]
        pub initial_page_number: Option<i32>,
        #[serde(rename = "@chart-size")]
        pub chart_size: Option<ChartSize>,
        #[serde(rename = "@margin-left")]
        pub margin_left: Option<Length>,
        #[serde(rename = "@margin-right")]
        pub margin_right: Option<Length>,
        #[serde(rename = "@margin-top")]
        pub margin_top: Option<Length>,
        #[serde(rename = "@margin-bottom")]
        pub margin_bottom: Option<Length>,
        #[serde(rename = "@paper-height")]
        pub paper_height: Option<Length>,
        #[serde(rename = "@paper-width")]
        pub paper_width: Option<Length>,
        #[serde(rename = "@reference-orientation")]
        pub reference_orientation: Option<String>,
        #[serde(rename = "@space-after")]
        pub space_after: Option<Length>,
        pub page_header: PageHeader,
        pub page_footer: PageFooter,
    }

    impl PageSetup {
        pub fn decode(
            self,
            warn: &mut dyn FnMut(Warning),
            structure_member: &str,
        ) -> page::PageSetup {
            let mut setup = page::PageSetup::default();
            if let Some(initial_page_number) = self.initial_page_number {
                setup.initial_page_number = initial_page_number;
            }
            if let Some(chart_size) = self.chart_size {
                setup.chart_size = chart_size.into();
            }
            if let Some(margin_left) = self.margin_left {
                setup.margins.0[Axis2::X][0] = margin_left.into();
            }
            if let Some(margin_right) = self.margin_right {
                setup.margins.0[Axis2::X][1] = margin_right.into();
            }
            if let Some(margin_top) = self.margin_top {
                setup.margins.0[Axis2::Y][0] = margin_top.into();
            }
            if let Some(margin_bottom) = self.margin_bottom {
                setup.margins.0[Axis2::Y][1] = margin_bottom.into();
            }
            match (self.paper_width, self.paper_height) {
                (Some(width), Some(height)) => {
                    setup.paper = PaperSize::new(width.0, height.0, paper_sizes::Unit::Inch)
                }
                (Some(length), None) | (None, Some(length)) => {
                    setup.paper = PaperSize::new(length.0, length.0, paper_sizes::Unit::Inch)
                }
                (None, None) => (),
            }
            if let Some(reference_orientation) = &self.reference_orientation {
                if reference_orientation.starts_with("0") {
                    setup.orientation = Orientation::Portrait;
                } else if reference_orientation.starts_with("90") {
                    setup.orientation = Orientation::Landscape;
                } else {
                    warn(Warning {
                        member: structure_member.into(),
                        details: WarningDetails::UnknownOrientation(reference_orientation.clone()),
                    });
                }
            }
            if let Some(space_after) = self.space_after {
                setup.object_spacing = space_after.into();
            }
            if let Some(PageParagraph { text }) = &self.page_header.page_paragraph {
                setup.header = text.decode();
            }
            if let Some(PageParagraph { text }) = &self.page_footer.page_paragraph {
                setup.footer = text.decode();
            }
            setup
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PageHeader {
        page_paragraph: Option<PageParagraph>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PageFooter {
        page_paragraph: Option<PageParagraph>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PageParagraph {
        text: PageParagraphText,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PageParagraphText {
        #[serde(default, rename = "$text")]
        text: String,
    }

    impl PageParagraphText {
        fn decode(&self) -> Document {
            Document::from_html(&self.text)
        }
    }

    #[derive(Copy, Clone, Debug, Default, Deserialize)]
    #[serde(rename = "snake_case")]
    enum ReferenceOrientation {
        #[serde(alias = "0")]
        #[serde(alias = "0deg")]
        #[serde(alias = "inherit")]
        #[default]
        Portrait,

        #[serde(alias = "90")]
        #[serde(alias = "90deg")]
        #[serde(alias = "-270")]
        #[serde(alias = "-270deg")]
        Landscape,

        #[serde(alias = "180")]
        #[serde(alias = "180deg")]
        #[serde(alias = "-1280")]
        #[serde(alias = "-180deg")]
        ReversePortrait,

        #[serde(alias = "270")]
        #[serde(alias = "270deg")]
        #[serde(alias = "-90")]
        #[serde(alias = "-90deg")]
        Seascape,
    }

    impl From<ReferenceOrientation> for page::Orientation {
        fn from(value: ReferenceOrientation) -> Self {
            match value {
                ReferenceOrientation::Portrait | ReferenceOrientation::ReversePortrait => {
                    page::Orientation::Portrait
                }
                ReferenceOrientation::Landscape | ReferenceOrientation::Seascape => {
                    page::Orientation::Landscape
                }
            }
        }
    }

    /// Chart size.
    #[derive(Copy, Clone, Debug, Default, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum ChartSize {
        FullHeight,
        HalfHeight,
        QuarterHeight,
        #[default]
        #[serde(other)]
        AsIs,
    }

    impl From<ChartSize> for page::ChartSize {
        fn from(value: ChartSize) -> Self {
            match value {
                ChartSize::AsIs => page::ChartSize::AsIs,
                ChartSize::FullHeight => page::ChartSize::FullHeight,
                ChartSize::HalfHeight => page::ChartSize::HalfHeight,
                ChartSize::QuarterHeight => page::ChartSize::QuarterHeight,
            }
        }
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    enum HeadingContent {
        Container(Container),
        Heading(Box<Heading>),
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Label {
        #[serde(default, rename = "$text")]
        text: String,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Container {
        #[serde(default, rename = "@visibility")]
        visibility: Visibility,
        #[serde(rename = "@page-break-before")]
        #[serde(default)]
        page_break_before: PageBreakBefore,
        #[serde(rename = "@text-align")]
        text_align: Option<TextAlign>,
        #[serde(rename = "@width")]
        width: Option<Length>,
        label: Label,

        #[serde(rename = "$value")]
        content: ContainerContent,
    }

    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "camelCase")]
    enum PageBreakBefore {
        #[default]
        Auto,
        Always,
        Avoid,
        Left,
        Right,
        Inherit,
    }

    #[derive(Deserialize, Debug, Default, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    enum Visibility {
        #[default]
        Visible,
        Hidden,
    }

    #[derive(Copy, Clone, Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    enum TextAlign {
        Left,
        Center,
        Right,
    }

    impl From<TextAlign> for HorzAlign {
        fn from(value: TextAlign) -> Self {
            match value {
                TextAlign::Left => HorzAlign::Left,
                TextAlign::Center => HorzAlign::Center,
                TextAlign::Right => HorzAlign::Right,
            }
        }
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    enum ContainerContent {
        Table(Table),
        Text(ContainerText),
        Graph(Graph),
        Model(Model),
        Object(Object),
        Image(Image),
        Tree(Tree),
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Graph {
        #[serde(rename = "@commandName")]
        command_name: String,

        /// Data.
        #[serde(rename = "dataPath")]
        data_member: Option<String>,

        /// XML in VizML format.
        #[serde(rename = "path")]
        xml_member: String,

        /// Rarely used, not understood.
        #[serde(rename = "csvPath")]
        csv_member: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Model {
        #[serde(rename = "@commandName")]
        command_name: String,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Tree {
        #[serde(rename = "@commandName")]
        command_name: String,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Image {
        #[serde(rename = "@commandName")]
        command_name: Option<String>,
        data_path: String,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Object {
        #[serde(rename = "@commandName")]
        command_name: Option<String>,
        #[serde(rename = "@uri")]
        uri: String,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Table {
        #[serde(rename = "@commandName")]
        command_name: String,
        #[serde(rename = "@subType")]
        subtype: String,
        #[serde(rename = "@tableId")]
        table_id: Option<i64>,
        #[serde(rename = "@type")]
        table_type: TableType,
        table_properties: Option<Box<TableProperties>>,
        table_structure: TableStructure,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct ContainerText {
        #[serde(rename = "@type")]
        text_type: TextType,
        #[serde(rename = "@commandName")]
        command_name: Option<String>,
        html: String,
    }

    impl ContainerText {
        fn decode(&self) -> Value {
            html::Document::from_html(&self.html).into_value()
        }
    }
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    enum TextType {
        Title,
        Log,
        Text,
        #[serde(rename = "page-title")]
        PageTitle,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct TableStructure {
        /// The `.xml` member name, for legacy members only.
        #[serde(rename = "path")]
        xml_member: Option<String>,
        /// The `.bin` member name.
        #[serde(rename = "dataPath")]
        bin_member: String,
        /// Rarely used, not understood.
        #[serde(rename = "csvPath")]
        _csv_member: Option<String>,
    }
}
