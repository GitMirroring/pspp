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

use anyhow::Result;
use binrw::{BinRead, error::ContextExt};
use clap::{Args, ValueEnum};
use pspp::{
    output::{
        Criteria, Item, Itemlike, SpvMembers,
        pivot::{Axis3, Dimension, Group, Leaf, PivotTable, value::Value},
    },
    spv::{
        SpvArchive,
        legacy_bin::LegacyBin,
        read::{
            ReadSeek,
            legacy_xml::{Visualization, datum_as_format},
            structure::OutlineItem,
        },
    },
};
use std::{
    fmt::Display,
    io::{BufReader, Cursor, Read},
    path::PathBuf,
    sync::Arc,
};

/// Show information about SPSS viewer files (SPV files).
#[derive(Args, Clone, Debug)]
pub struct ShowSpv {
    /// What to show.
    #[arg(value_enum)]
    mode: Mode,

    /// File to show.
    ///
    /// For most modes, this should be a `.spv` file.  For `convert-table-look`,
    /// this should be a `.tlo` or `.stt` file.
    #[arg(required = true)]
    input: PathBuf,

    /// Password for decryption.
    ///
    /// In addition to file encryption, SPSS supports a feature called "password
    /// encryption".  The password specified can be specified with or without
    /// "password encryption".
    ///
    /// Specify only for an encrypted SPV file.
    #[clap(short, long)]
    password: Option<String>,

    /// Input selection options.
    #[command(flatten)]
    criteria: Criteria,

    /// Include ZIP member names in `dir` output.
    #[arg(long = "member-names")]
    show_member_names: bool,
}

/// What to show in a viewer file.
#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
enum Mode {
    /// List tables and other items.
    #[value(alias = "dir")]
    Directory,

    /// Copies first selected TableLook into output in `.stt` format.
    GetTableLook,

    /// Reads `.tlo` or `.stt` TableLook and outputs as `.stt` format.
    ConvertTableLook,

    /// Print data values in legacy tables.
    ///
    /// Data values come from `_tableData.bin` members inside the SPV files.
    /// They do not require reading the corresponding `_table.xml` files.
    LegacyData,

    /// Print data series in legacy tables.
    ///
    /// The series come from `_tableData.bin` members inside the SPV files, as
    /// transformed by instructions in their paired `_table.xml` files.
    LegacySeries,

    /// Prints contents.
    View,
}

impl Mode {
    fn as_str(&self) -> &'static str {
        match self {
            Mode::Directory => "directory",
            Mode::GetTableLook => "get-table-look",
            Mode::ConvertTableLook => "convert-table-look",
            Mode::LegacyData => "legacy-data",
            Mode::LegacySeries => "legacy-series",
            Mode::View => "view",
        }
    }
}

impl Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl ShowSpv {
    pub fn run(self) -> Result<()> {
        match self.mode {
            Mode::Directory => self.directory(),
            Mode::View => self.view(),
            Mode::LegacyData => self.legacy_data(),
            Mode::LegacySeries => self.legacy_series(),
            Mode::GetTableLook => todo!(),
            Mode::ConvertTableLook => todo!(),
        }
    }

    fn read(&self) -> Result<Vec<Arc<Item>>> {
        Ok(self.criteria.apply(
            SpvArchive::open_file(&self.input, self.password.as_deref())?
                .read(|e| eprintln!("{e}"))?
                .into_items(),
        ))
    }

    fn read_outline(&self) -> Result<(SpvArchive<Box<dyn ReadSeek>>, Vec<Arc<OutlineItem>>)> {
        let mut archive = SpvArchive::open_file(&self.input, self.password.as_deref())?;
        let outline = archive.read_outline(|w| eprintln!("{w}"))?;
        Ok((archive, self.criteria.apply(outline.items)))
    }

    fn directory(self) -> Result<()> {
        for child in self.read_outline()?.1 {
            print_item_directory(&*child, 0, self.show_member_names);
        }
        Ok(())
    }

    fn view(self) -> Result<()> {
        for child in self.read()? {
            println!("{child}");
        }
        Ok(())
    }

    fn legacy_data(self) -> Result<()> {
        let (mut archive, items) = self.read_outline()?;
        for item in items {
            for item in item.iter_in_order() {
                if let Some(spv_info) = item.spv_info()
                    && let Some(members) = &spv_info.members
                    && let Some(binary) = members.bin_member()
                {
                    let mut bin_member = archive.0.by_name(binary)?;
                    let mut bin_data = Vec::with_capacity(bin_member.size() as usize);
                    bin_member.read_to_end(&mut bin_data)?;
                    let mut cursor = Cursor::new(bin_data);
                    let legacy_bin = LegacyBin::read(&mut cursor).map_err(|e| {
                        e.with_message(format!(
                            "While parsing {binary:?} as legacy binary SPV member"
                        ))
                    })?;
                    let data = legacy_bin.decode(&mut |w| eprintln!("{w}"));
                    let n_values = data
                        .values()
                        .flat_map(|map| map.values())
                        .map(|values| values.len())
                        .max()
                        .unwrap_or(0);
                    let index = Dimension::new(
                        Group::new("Index")
                            .with_multiple(Leaf::numbers(0..n_values))
                            .with_label_shown(),
                    );
                    let variables = Dimension::new(Group::new("Variables").with_multiple(
                        data.iter().map(|(name, contents)| {
                            Group::new(name.as_str()).with_multiple(contents.keys().map(|name| {
                                name.replace("categories", "\ncategories")
                                    .replace("labels", "\nlabels")
                                    .replace("group", "\ngroup")
                                    .replace("Label", "\nLabel")
                            }))
                        }),
                    ));
                    let mut pivot_table =
                        PivotTable::new([(Axis3::Y, index), (Axis3::X, variables)]);
                    for (variable_index, (variable_name, values)) in
                        data.values().flat_map(|map| map.iter()).enumerate()
                    {
                        for (value_index, data_value) in values.iter().enumerate() {
                            let value = Value::new_datum(data_value).with_value_label(
                                (variable_name == "cellFormat")
                                    .then(|| datum_as_format(data_value).to_string()),
                            );
                            pivot_table.insert([value_index, variable_index], value);
                        }
                    }
                    println!("{pivot_table}");
                }
            }
        }
        Ok(())
    }

    fn legacy_series(self) -> Result<()> {
        let (mut archive, items) = self.read_outline()?;
        for item in items {
            for item in item.iter_in_order() {
                if let Some(spv_info) = item.spv_info()
                    && let Some(members) = &spv_info.members
                    && let SpvMembers::LegacyTable { xml, binary } = &members
                {
                    // Read and decode binary file.
                    let mut bin_member = archive.0.by_name(&binary)?;
                    let mut bin_data = Vec::with_capacity(bin_member.size() as usize);
                    bin_member.read_to_end(&mut bin_data)?;
                    let mut cursor = Cursor::new(bin_data);
                    let legacy_bin = LegacyBin::read(&mut cursor).map_err(|e| {
                        e.with_message(format!(
                            "While parsing {binary:?} as legacy binary SPV member"
                        ))
                    })?;
                    let data = legacy_bin.decode(&mut |w| eprintln!("{w}"));
                    drop(bin_member);

                    // Read and decode series in XML file.
                    let member = BufReader::new(archive.0.by_name(&xml)?);
                    let visualization: Visualization = match serde_path_to_error::deserialize(
                        &mut quick_xml::de::Deserializer::from_reader(member),
                    ) {
                        Ok(result) => result,
                        Err(error) => panic!("{error:?}"),
                    };
                    let series = visualization.decode_series(data, &mut |w| {
                        eprintln!("{w}");
                    });

                    let n_values = series
                        .values()
                        .map(|map| map.values.len())
                        .max()
                        .unwrap_or(0);
                    let index = Dimension::new(
                        Group::new("Index")
                            .with_multiple(Leaf::numbers(0..n_values))
                            .with_label_shown(),
                    );
                    let variables = Dimension::new(Group::new("Series").with_multiple(
                        series.values().map(|series| {
                            series
                                .name
                                .replace("categories", "\ncategories")
                                .replace("labels", "\nlabels")
                                .replace("group", "\ngroup")
                                .replace("Label", "\nLabel")
                        }),
                    ));
                    let mut pivot_table =
                        PivotTable::new([(Axis3::Y, index), (Axis3::X, variables)]);
                    for (series_index, series) in series.values().enumerate() {
                        for (value_index, data_value) in series.values.iter().enumerate() {
                            pivot_table
                                .insert([value_index, series_index], Value::new_datum(data_value));
                        }
                    }
                    println!("{pivot_table}");
                }
            }
        }
        Ok(())
    }
}

fn print_item_directory<T>(item: &T, level: usize, show_member_names: bool)
where
    T: Itemlike,
{
    for _ in 0..level {
        print!("    ");
    }
    print!("- {} {:?}", item.kind(), item.label());
    /*
    if let Some(table) = item.details.as_table() {
        let title = table.title().display(table).to_string();
        if item.label.as_ref().is_none_or(|label| label != &title) {
            print!(" title {title:?}");
        }
    }*/
    if let Some(command_name) = item.command_name() {
        print!(" command {command_name:?}");
    }
    if let Some(subtype) = item.subtype()
        && let label = item.label().as_ref()
        && label != &subtype
    {
        print!(" subtype {subtype:?}");
    }
    if item.is_expanded() == Some(false) {
        print!(" (collapsed");
    }
    if item.is_shown() == Some(false) {
        print!(" (hidden)");
    }
    if show_member_names && let Some(spv_info) = item.spv_info() {
        for (index, name) in spv_info.member_names().into_iter().enumerate() {
            print!(" {} {name:?}", if index == 0 { "in" } else { "and" });
        }
    }
    println!();
    for child in item.children() {
        print_item_directory(&**child, level + 1, show_member_names);
    }
}
