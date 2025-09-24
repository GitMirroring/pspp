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

use super::parse_encoding;
use anyhow::{Result, anyhow};
use clap::{Args, ValueEnum};
use encoding_rs::Encoding;
use itertools::Itertools;
use pspp::{
    data::cases_to_output,
    output::{Details, Item, Text, drivers::Driver, pivot::PivotTable},
    sys::{
        Records,
        raw::{Decoder, EncodingReport, Magic, Reader, Record, infer_encoding},
    },
};
use serde::Serialize;
use std::{cell::RefCell, fmt::Display, fs::File, io::BufReader, path::PathBuf, rc::Rc, sync::Arc};

/// Show information about SPSS system files.
#[derive(Args, Clone, Debug)]
pub struct Show {
    /// What to show.
    #[arg(value_enum)]
    mode: Mode,

    /// File to show.
    #[arg(required = true)]
    input: PathBuf,

    /// Output file name.  If omitted, output is written to stdout.
    output: Option<PathBuf>,

    /// The encoding to use.
    #[arg(long, value_parser = parse_encoding, help_heading = "Input file options")]
    encoding: Option<&'static Encoding>,

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
    max_cases: u64,

    /// Output driver configuration options.
    #[arg(short = 'o', help_heading = "Output options")]
    output_options: Vec<String>,
}

struct Output {
    driver: Rc<RefCell<Box<dyn Driver>>>,
    mode: Mode,
}

impl Output {
    fn show<T>(&self, value: &T) -> Result<()>
    where
        T: Serialize,
        for<'a> &'a T: Into<Details>,
    {
        let mut driver = self.driver.borrow_mut();
        if driver.can_serialize() {
            driver.serialize(value);
        } else {
            driver.write(&Arc::new(value.into().into_item()));
        }
        Ok(())
    }

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

impl Show {
    pub fn run(self) -> Result<()> {
        let output = Output {
            mode: self.mode,
            driver: Rc::new(RefCell::new(Box::new(<dyn Driver>::from_options(
                self.output.as_ref(),
                &self.output_options,
                "json",
            )?))),
        };

        let reader = File::open(&self.input)?;
        let reader = BufReader::new(reader);
        let mut reader = Reader::new(reader, Box::new(|warning| output.warn(&warning)))?;

        match self.mode {
            Mode::Identity => {
                match reader.header().magic {
                    Magic::Sav => println!("SPSS System File"),
                    Magic::Zsav => println!("SPSS System File with Zlib compression"),
                    Magic::Ebcdic => println!("EBCDIC-encoded SPSS System File"),
                }
                return Ok(());
            }
            Mode::Raw => {
                output.show_json(reader.header())?;
                for record in reader.records() {
                    output.show_json(&record?)?;
                }
                for (_index, case) in (0..self.max_cases).zip(reader.cases()) {
                    output.show_json(&case?)?;
                }
            }
            Mode::Decoded => {
                let records: Vec<Record> = reader.records().collect::<Result<Vec<_>, _>>()?;
                let encoding = match self.encoding {
                    Some(encoding) => encoding,
                    None => infer_encoding(&records, &mut |e| output.warn(&e))?,
                };
                let mut decoder = Decoder::new(encoding, |e| output.warn(&e));
                for record in records {
                    output.show_json(&record.decode(&mut decoder))?;
                }
            }
            Mode::Dictionary => {
                let records: Vec<Record> = reader.records().collect::<Result<Vec<_>, _>>()?;
                let encoding = match self.encoding {
                    Some(encoding) => encoding,
                    None => infer_encoding(&records, &mut |e| output.warn(&e))?,
                };
                let mut decoder = Decoder::new(encoding, |e| output.warn(&e));
                let records = Records::from_raw(records, &mut decoder);
                let (dictionary, metadata, cases) = records
                    .decode(
                        reader.header().clone().decode(&mut decoder),
                        reader.cases(),
                        encoding,
                        |e| output.warn(&e),
                    )
                    .into_parts();

                if output.can_show_json() {
                    output.show_json(&dictionary)?;
                    output.show_json(&metadata)?;
                    for (_index, case) in (0..self.max_cases).zip(cases) {
                        output.show_json(&case?)?;
                    }
                } else {
                    let mut items = Vec::new();
                    items.push(PivotTable::from(&metadata).into());
                    items.extend(dictionary.all_pivot_tables().into_iter().map_into());
                    items.extend(cases_to_output(&dictionary, cases));
                    output
                        .driver
                        .borrow_mut()
                        .write(&Arc::new(items.into_iter().collect()));
                }
            }
            Mode::Encodings => {
                let encoding_report = EncodingReport::new(reader, self.max_cases)?;
                output.show(&encoding_report)?;
            }
        }

        Ok(())
    }
}

/// What to show in a system file.
#[derive(Clone, Copy, Debug, Default, PartialEq, ValueEnum)]
enum Mode {
    /// The kind of file.
    Identity,

    /// File dictionary, with variables, value labels, attributes, ...
    #[default]
    #[value(alias = "dict")]
    Dictionary,

    /// Possible encodings of text in file dictionary and (with `--data`) cases.
    Encodings,

    /// Raw file records, without assuming a particular character encoding.
    Raw,

    /// Raw file records decoded with a particular character encoding.
    Decoded,
}

impl Mode {
    fn as_str(&self) -> &'static str {
        match self {
            Mode::Dictionary => "dictionary",
            Mode::Identity => "identity",
            Mode::Raw => "raw",
            Mode::Decoded => "decoded",
            Mode::Encodings => "encodings",
        }
    }
}

impl Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
