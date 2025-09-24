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
    fmt::Display,
    fs::File,
    io::{BufWriter, Error, Write, stdout},
    path::PathBuf,
    sync::Arc,
};

use chrono::{Datelike, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};

use crate::{
    calendar::calendar_offset_to_gregorian,
    data::{ByteString, Case, Datum, WithEncoding},
    dictionary::Dictionary,
    format::{DisplayPlain, Type},
    output::{Item, drivers::Driver, table::CellPos},
    util::ToSmallString as _,
    variable::Variable,
};

use crate::output::{Details, TextType, pivot::PivotTable, table::Table};

use super::CaseWriter;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CsvConfig {
    file: Option<PathBuf>,
    #[serde(flatten)]
    options: CsvOptions,
}

pub struct CsvDriver {
    file: Box<dyn Write>,
    options: CsvOptions,

    /// Number of items written so far.
    n_items: usize,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct CsvOptions {
    quote: char,
    delimiter: char,

    /// Omit writing variable names as the first line of output.
    var_names: bool,

    /// Writes user-missing values like system-missing values.  Otherwise,
    /// user-missing values are written the same way as non-missing values.
    recode: bool,

    /// Write value labels instead of values.
    labels: bool,

    /// Use print formats for numeric variables.
    print_formats: bool,

    /// Decimal point.
    decimal: char,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            quote: '"',
            delimiter: ',',
            var_names: true,
            recode: false,
            labels: false,
            print_formats: false,
            decimal: '.',
        }
    }
}

impl CsvOptions {
    fn field<'a>(&'a self, text: &'a str) -> CsvField<'a> {
        CsvField::new(text, self)
    }

    fn write_field<W>(
        &self,
        datum: &Datum<WithEncoding<ByteString>>,
        variable: &Variable,
        file: &mut W,
    ) -> std::io::Result<()>
    where
        W: Write,
    {
        if self.labels
            && let Some(label) = variable.value_labels.get(datum)
        {
            write!(file, "{}", self.field(label))
        } else if datum.is_sysmis() || (self.recode && variable.missing_values().contains(datum)) {
            write!(file, "{}", self.field(" "))
        } else if self.print_formats || datum.is_string() {
            write!(
                file,
                "{}",
                self.field(
                    &datum
                        .display(variable.print_format)
                        .without_spaces()
                        .to_small_string::<64>(),
                )
            )
        } else {
            let number = datum.as_number().unwrap().unwrap();
            match variable.print_format.type_() {
                Type::F
                | Type::Comma
                | Type::Dot
                | Type::Dollar
                | Type::Pct
                | Type::E
                | Type::CC(_)
                | Type::N
                | Type::Z
                | Type::P
                | Type::PK
                | Type::IB
                | Type::PIB
                | Type::PIBHex
                | Type::RB
                | crate::format::Type::RBHex
                | Type::WkDay
                | Type::Month => write!(
                    file,
                    "{}",
                    self.field(
                        &number
                            .display_plain()
                            .with_decimal(self.decimal)
                            .to_small_string::<64>()
                    )
                ),

                Type::Date
                | Type::ADate
                | Type::EDate
                | Type::JDate
                | Type::SDate
                | Type::QYr
                | Type::MoYr
                | Type::WkYr => {
                    if number >= 0.0
                        && let Some(date) =
                            calendar_offset_to_gregorian(number / 60.0 / 60.0 / 24.0)
                    {
                        write!(
                            file,
                            "{}",
                            self.field(
                                &format_args!(
                                    "{:02}/{:02}/{:04}",
                                    date.month(),
                                    date.day(),
                                    date.year()
                                )
                                .to_small_string::<64>()
                            )
                        )
                    } else {
                        write!(file, "{}", self.field(" "))
                    }
                }

                Type::DateTime | Type::YmdHms => {
                    if number >= 0.0
                        && let Some(date) =
                            calendar_offset_to_gregorian(number / 60.0 / 60.0 / 24.0)
                        && let Some(time) = NaiveTime::from_num_seconds_from_midnight_opt(
                            (number % (60.0 * 60.0 * 24.0)) as u32,
                            0,
                        )
                    {
                        write!(
                            file,
                            "{}",
                            self.field(
                                &format_args!(
                                    "{:02}/{:02}/{:04} {:02}:{:02}:{:02}",
                                    date.month(),
                                    date.day(),
                                    date.year(),
                                    time.hour(),
                                    time.minute(),
                                    time.second()
                                )
                                .to_small_string::<64>(),
                            )
                        )
                    } else {
                        write!(file, "{}", self.field(" "))
                    }
                }

                Type::MTime | Type::Time | Type::DTime => {
                    if let Some(time) =
                        NaiveTime::from_num_seconds_from_midnight_opt(number.abs() as u32, 0)
                    {
                        write!(
                            file,
                            "{}",
                            self.field(
                                &format_args!(
                                    "{}{:02}:{:02}:{:02}",
                                    if number.is_sign_negative() { "-" } else { "" },
                                    time.hour(),
                                    time.minute(),
                                    time.second()
                                )
                                .to_small_string::<64>(),
                            )
                        )
                    } else {
                        write!(file, "{}", self.field(" "))
                    }
                }

                Type::A | Type::AHex => unreachable!(),
            }
        }
    }
}

struct CsvField<'a> {
    text: &'a str,
    delimiter: char,
    quote: char,
}

impl<'a> CsvField<'a> {
    fn new(text: &'a str, options: &CsvOptions) -> Self {
        Self {
            text,
            delimiter: options.delimiter,
            quote: options.quote,
        }
    }

    fn char_needs_quoting(&self, b: char) -> bool {
        b == '\r' || b == '\n' || b == self.quote || b == self.delimiter
    }

    fn needs_quoting(&self) -> bool {
        self.text.chars().any(|b| self.char_needs_quoting(b))
    }
}

impl Display for CsvField<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.needs_quoting() {
            let quote = self.quote;
            write!(f, "{quote}")?;
            for c in self.text.chars() {
                if c == quote {
                    write!(f, "{c}")?;
                }
                write!(f, "{c}")?;
            }
            write!(f, "{quote}")
        } else {
            write!(f, "{}", self.text)
        }
    }
}

impl CsvDriver {
    pub fn new(config: &CsvConfig) -> std::io::Result<Self> {
        Ok(Self {
            file: match &config.file {
                Some(file) => Box::new(BufWriter::new(File::create(file)?)),
                None => Box::new(stdout()),
            },
            options: config.options,
            n_items: 0,
        })
    }

    fn start_item(&mut self) {
        if self.n_items > 0 {
            writeln!(&mut self.file).unwrap();
        }
        self.n_items += 1;
    }

    fn output_table_layer(&mut self, pt: &PivotTable, layer: &[usize]) -> Result<(), Error> {
        let output = pt.output(layer, true);
        self.start_item();

        self.output_table(pt, output.title.as_ref(), Some("Table"))?;
        for (index, layer) in output.layers.iter().enumerate() {
            self.output_table(pt, Some(layer), (index == 0).then_some("Layer"))?;
        }
        self.output_table(pt, Some(&output.body), None)?;
        self.output_table(pt, output.caption.as_ref(), Some("Caption"))?;
        self.output_table(pt, output.footnotes.as_ref(), Some("Footnote"))?;
        Ok(())
    }

    fn output_table(
        &mut self,
        pivot_table: &PivotTable,
        table: Option<&Table>,
        leader: Option<&str>,
    ) -> Result<(), Error> {
        let Some(table) = table else {
            return Ok(());
        };

        for y in 0..table.n.y {
            for x in 0..table.n.x {
                if x > 0 {
                    write!(&mut self.file, "{}", self.options.delimiter)?;
                }

                let coord = CellPos { x, y };
                let content = table.get(coord);
                if content.is_top_left() {
                    let display = content.inner().value.display(pivot_table);
                    let s = match leader {
                        Some(leader) if x == 0 && y == 0 => format!("{leader}: {display}"),
                        _ => display.to_string(),
                    };
                    write!(&mut self.file, "{}", CsvField::new(&s, &self.options))?;
                }
            }
            writeln!(&mut self.file)?;
        }

        Ok(())
    }
}

impl Driver for CsvDriver {
    fn name(&self) -> Cow<'static, str> {
        Cow::from("csv")
    }

    fn write(&mut self, item: &Arc<Item>) {
        // todo: error handling (should not unwrap)
        match &item.details {
            Details::Graph | Details::Image(_) | Details::Heading(_) => (),
            Details::Message(diagnostic) => {
                self.start_item();
                let text = diagnostic.to_string();
                writeln!(&mut self.file, "{}", CsvField::new(&text, &self.options)).unwrap();
            }
            Details::Table(pivot_table) => {
                for layer in pivot_table.layers(true) {
                    self.output_table_layer(pivot_table, &layer).unwrap();
                }
            }
            Details::Text(text) => match text.type_ {
                TextType::Syntax | TextType::PageTitle => (),
                TextType::Title | TextType::Log => {
                    self.start_item();
                    for line in text.content.display(()).to_string().lines() {
                        writeln!(&mut self.file, "{}", CsvField::new(line, &self.options)).unwrap();
                    }
                }
            },
        }
    }

    fn flush(&mut self) {
        let _ = self.file.flush();
    }

    fn can_write_data_file(&self) -> bool {
        true
    }

    fn write_data_file<'a>(
        &'a mut self,
        dictionary: &'a Dictionary,
    ) -> anyhow::Result<Option<Box<dyn CaseWriter + 'a>>> {
        for (index, variable) in dictionary.variables.iter().enumerate() {
            if index > 0 {
                write!(&mut self.file, "{}", self.options.delimiter)?;
            }
            let name = variable.name.as_str();
            write!(&mut self.file, "{}", CsvField::new(name, &self.options))?;
        }
        writeln!(&mut self.file)?;
        Ok(Some(Box::new(CsvDriverCaseWriter {
            driver: self,
            dictionary,
        })))
    }
}

struct CsvDriverCaseWriter<'a> {
    driver: &'a mut CsvDriver,
    dictionary: &'a Dictionary,
}

impl<'a> CaseWriter for CsvDriverCaseWriter<'a> {
    fn write_case(&mut self, case: Case<Vec<Datum<ByteString>>>) -> anyhow::Result<()> {
        for (datum, variable) in case.into_iter().zip(self.dictionary.variables.iter()) {
            self.driver
                .options
                .write_field(&datum, variable, &mut self.driver.file)?;
        }
        writeln!(&mut self.driver.file).unwrap();
        Ok(())
    }
}
