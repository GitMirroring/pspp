// PSPP - a program for statistical analysis.
// Copyright (C) 2025 Free Software Foundation, Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::{path::PathBuf, sync::Arc};

use anyhow::{Error as AnyError, Result, bail};
use clap::Args;
use encoding_rs::Encoding;
use pspp::{
    data::{ByteString, Case, Datum},
    dictionary::Dictionary,
    file::FileType,
    output::{Criteria, drivers::Driver},
    pc::PcFile,
    por::PortableFile,
    spv::SpvArchive,
    sys::ReadOptions,
};

use super::parse_encoding;

/// Convert SPSS data and viewer files into other formats.
#[derive(Args, Clone, Debug)]
pub struct Convert {
    /// Input file name.
    input: PathBuf,

    /// Output file name (if omitted, output is written to stdout).
    output: Option<PathBuf>,

    /// The encoding to use for reading the input file.
    #[arg(short = 'e', long, value_parser = parse_encoding)]
    encoding: Option<&'static Encoding>,

    /// Password for decryption.
    ///
    /// In addition to file encryption, SPSS supports a feature called "password
    /// encryption".  The password specified can be specified with or without
    /// "password encryption".
    ///
    /// Specify only for an encrypted system file.
    #[clap(short, long)]
    password: Option<String>,

    /// Maximum number of cases to print.
    #[arg(short = 'c', long = "cases")]
    max_cases: Option<usize>,

    /// Write the output file with Unicode (UTF-8) encoding.
    ///
    /// If the input was not already encoded in Unicode, this triples the width
    /// of string variables.
    #[arg(long = "unicode")]
    to_unicode: bool,

    /// Output driver configuration options.
    #[arg(short = 'o')]
    output_options: Vec<String>,

    /// Selection options for SPV input files.
    #[command(flatten)]
    criteria: Criteria,
}

impl Convert {
    fn open_driver(&self, default_driver: &str) -> Result<Box<dyn Driver>> {
        <dyn Driver>::from_options(self.output.as_ref(), &self.output_options, default_driver)
    }
    fn write_data(
        self,
        dictionary: Dictionary,
        cases: Box<dyn Iterator<Item = Result<Case<Vec<Datum<ByteString>>>, AnyError>>>,
    ) -> Result<()> {
        // Take only the first `self.max_cases` cases.
        let cases = cases.take(self.max_cases.unwrap_or(usize::MAX));

        let mut output = self.open_driver("csv")?;
        if !output.can_write_data_file() {
            bail!("Can't write data output to {} driver.", output.name());
        }

        let mut writer = output.write_data_file(&dictionary)?.unwrap();
        for case in cases {
            writer.write_case(case?)?;
        }
        Ok(())
    }

    pub fn run(self) -> Result<()> {
        match FileType::from_file(&self.input)? {
            Some(FileType::System { .. }) => {
                fn warn(warning: anyhow::Error) {
                    eprintln!("warning: {warning}");
                }

                let mut system_file = ReadOptions::new(warn)
                    .with_encoding(self.encoding)
                    .with_password(self.password.clone())
                    .open_file(&self.input)?;
                if self.to_unicode {
                    system_file = system_file.into_unicode();
                }
                let (dictionary, _, cases) = system_file.into_parts();
                let cases = cases.map(|result| result.map_err(AnyError::from));
                let cases = Box::new(cases)
                    as Box<dyn Iterator<Item = Result<Case<Vec<Datum<ByteString>>>, AnyError>>>;
                self.write_data(dictionary, cases)
            }
            Some(FileType::Portable) => {
                fn warn_portable(warning: pspp::por::Warning) {
                    eprintln!("warning: {warning}");
                }

                let portable_file = PortableFile::open_file(&self.input, warn_portable)?;
                let (dictionary, _, cases) = portable_file.into_parts();
                let cases = cases.map(|result| result.map_err(AnyError::from));
                let cases = Box::new(cases)
                    as Box<dyn Iterator<Item = Result<Case<Vec<Datum<ByteString>>>, AnyError>>>;
                self.write_data(dictionary, cases)
            }
            Some(FileType::Pc) => {
                fn warn_pc(warning: pspp::pc::Warning) {
                    eprintln!("warning: {warning}");
                }

                let pc_file = PcFile::open_file(&self.input, warn_pc)?;
                let (dictionary, _, cases) = pc_file.into_parts();
                let cases = cases.map(|result| result.map_err(AnyError::from));
                let cases = Box::new(cases)
                    as Box<dyn Iterator<Item = Result<Case<Vec<Datum<ByteString>>>, AnyError>>>;
                self.write_data(dictionary, cases)
            }
            Some(FileType::Viewer { .. }) => {
                let (items, page_setup) =
                    SpvArchive::open_file(&self.input, self.password.as_deref())?
                        .read(|e| eprintln!("{e}"))?
                        .into_contents();
                let mut output = self.open_driver("text")?;
                if let Some(page_setup) = &page_setup {
                    output.setup(page_setup);
                }
                for item in items {
                    output.write(&Arc::new(item));
                }
                Ok(())
            }
            _ => bail!(
                "{}: not a system, portable, or SPSS/PC+ file",
                self.input.display()
            ),
        }
    }
}
