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

use anyhow::{Result, anyhow};
use clap::{Args, ValueEnum};
use itertools::Itertools;
use pspp::{
    data::cases_to_output,
    output::{Item, Text, drivers::Driver, pivot::PivotTable},
    por::PortableFile,
};
use serde::Serialize;
use std::{cell::RefCell, fmt::Display, fs::File, io::BufReader, path::PathBuf, rc::Rc, sync::Arc};

/// Show information about SPSS portable files.
#[derive(Args, Clone, Debug)]
pub struct ShowPor {
    /// What to show.
    #[arg(value_enum)]
    mode: Mode,

    /// File to show.
    #[arg(required = true)]
    input: PathBuf,

    /// Output file name.  If omitted, output is written to stdout.
    output: Option<PathBuf>,

    /// Maximum number of cases to read.
    ///
    /// If specified without an argument, all cases will be read.
    #[arg(
        long = "data",
        num_args = 0..=1,
        default_missing_value = "18446744073709551615",
        default_value_t = 0,
        help_heading = "Input file options"
    )]
    max_cases: usize,

    /// Output driver configuration options.
    #[arg(short = 'o', help_heading = "Output options")]
    output_options: Vec<String>,
}

struct Output {
    driver: Rc<RefCell<Box<dyn Driver>>>,
    mode: Mode,
}

impl Output {
    fn can_show_json(&self) -> bool {
        self.driver.borrow().can_serialize()
    }

    fn show_json<T>(&self, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let mut driver = self.driver.borrow_mut();
        if driver.can_serialize() {
            driver.serialize(value);
            Ok(())
        } else {
            Err(anyhow!(
                "Mode '{}' only supports output as JSON.",
                self.mode
            ))
        }
    }

    fn warn(&self, warning: &impl Display) {
        let mut driver = self.driver.borrow_mut();
        #[derive(Serialize)]
        struct Warning {
            warning: String,
        }
        let w = Warning {
            warning: warning.to_string(),
        };
        if driver.can_serialize() {
            driver.serialize(&w);
        } else {
            driver.write(&Arc::new(Item::from(Text::new_log(warning.to_string()))));
        }
    }
}

impl ShowPor {
    pub fn run(self) -> Result<()> {
        let output = Output {
            mode: self.mode,
            driver: Rc::new(RefCell::new(Box::new(<dyn Driver>::from_options(
                self.output.as_ref(),
                &self.output_options,
                "json",
            )?))),
        };

        let reader = BufReader::new(File::open(&self.input)?);
        match self.mode {
            Mode::Dictionary => {
                let PortableFile {
                    dictionary,
                    metadata: _,
                    cases,
                } = PortableFile::open(reader, |warning| output.warn(&warning))?;
                let cases = cases.take(self.max_cases);

                if output.can_show_json() {
                    output.show_json(&dictionary)?;
                    for (_index, case) in (0..self.max_cases).zip(cases) {
                        output.show_json(&case?)?;
                    }
                } else {
                    let mut items = Vec::new();
                    items.extend(dictionary.all_pivot_tables().into_iter().map_into());
                    items.extend(cases_to_output(&dictionary, cases));
                    output
                        .driver
                        .borrow_mut()
                        .write(&Arc::new(items.into_iter().collect()));
                }
            }
            Mode::Metadata => {
                let metadata =
                    PortableFile::open(reader, |warning| output.warn(&warning))?.metadata;

                if output.can_show_json() {
                    output.show_json(&metadata)?;
                } else {
                    output
                        .driver
                        .borrow_mut()
                        .write(&Arc::new(PivotTable::from(&metadata).into()));
                }
            }
            Mode::Histogram => {
                let (histogram, translations) = PortableFile::read_histogram(reader)?;
                let h = histogram
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, count)| {
                        if count > 0
                            && index != translations[index as u8] as usize
                            && translations[index as u8] != 0
                        {
                            Some((
                                format!("{index:02x}"),
                                translations[index as u8] as char,
                                count,
                            ))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                output.show_json(&h)?;
            }
        }
        Ok(())
    }
}

/// What to show in a system file.
#[derive(Clone, Copy, Debug, Default, PartialEq, ValueEnum)]
enum Mode {
    /// File dictionary, with variables, value labels, ...
    #[default]
    #[value(alias = "dict")]
    Dictionary,

    /// File metadata not included in the dictionary.
    Metadata,

    /// Histogram of character incidence in the file.
    Histogram,
}

impl Mode {
    fn as_str(&self) -> &'static str {
        match self {
            Mode::Dictionary => "dictionary",
            Mode::Metadata => "metadata",
            Mode::Histogram => "histogram",
        }
    }
}

impl Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
