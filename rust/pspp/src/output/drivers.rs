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

use std::{borrow::Cow, fmt::Write, path::Path, sync::Arc};

use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::{
    data::{ByteString, Case, Datum},
    dictionary::Dictionary,
};

use super::{Item, page::PageSetup};

pub mod cairo;
use cairo::{CairoConfig, CairoDriver};

pub mod csv;
use csv::{CsvConfig, CsvDriver};

pub mod html;
use html::{HtmlConfig, HtmlDriver};

pub mod json;
use json::{JsonConfig, JsonDriver};

pub mod por;
use por::{PorConfig, PorDriver};

pub mod sav;
use sav::{SavConfig, SavDriver};

pub mod spv;
use spv::{SpvConfig, SpvDriver};

pub mod text;
use text::{TextConfig, TextDriver};

// An output driver.
pub trait Driver {
    fn name(&self) -> Cow<'static, str>;

    fn write(&mut self, item: &Arc<Item>);

    fn can_serialize(&self) -> bool {
        false
    }

    fn serialize(&mut self, item: &dyn erased_serde::Serialize) {
        let _ = item;
        unreachable!("This driver does not support serialization");
    }

    fn can_write_data_file(&self) -> bool {
        false
    }

    fn write_data_file<'a>(
        &'a mut self,
        dictionary: &'a Dictionary,
    ) -> anyhow::Result<Option<Box<dyn CaseWriter + 'a>>> {
        let _ = dictionary;
        Ok(None)
    }

    /// Returns false if the driver doesn't support page setup.
    fn setup(&mut self, page_setup: &PageSetup) -> bool {
        let _ = page_setup;
        false
    }

    /// Ensures that anything written with [Self::write] has been displayed.
    ///
    /// This is called from the text-based UI before showing the command prompt,
    /// to ensure that the user has actually been shown any preceding output If
    /// it doesn't make sense for this driver to be used this way, then this
    /// function need not do anything.
    fn flush(&mut self) {}

    /// Ordinarily, the core driver code will skip passing hidden output items
    /// to [Self::write].  If this returns true, the core driver hands them to
    /// the driver to let it handle them itself.
    fn handles_show(&self) -> bool {
        false
    }

    /// Ordinarily, the core driver code will flatten groups of output items
    /// before passing them to [Self::write].  If this returns true, the core
    /// driver code leaves them in place for the driver to handle.
    fn handles_groups(&self) -> bool {
        false
    }
}

impl Driver for Box<dyn Driver> {
    fn name(&self) -> Cow<'static, str> {
        (**self).name()
    }

    fn write(&mut self, item: &Arc<Item>) {
        (**self).write(item);
    }

    fn setup(&mut self, page_setup: &PageSetup) -> bool {
        (**self).setup(page_setup)
    }

    fn flush(&mut self) {
        (**self).flush();
    }

    fn handles_show(&self) -> bool {
        (**self).handles_show()
    }

    fn handles_groups(&self) -> bool {
        (**self).handles_groups()
    }

    fn can_serialize(&self) -> bool {
        (**self).can_serialize()
    }

    fn serialize(&mut self, item: &dyn erased_serde::Serialize) {
        (**self).serialize(item);
    }

    fn can_write_data_file(&self) -> bool {
        (**self).can_write_data_file()
    }

    fn write_data_file<'a>(
        &'a mut self,
        dictionary: &'a Dictionary,
    ) -> anyhow::Result<Option<Box<dyn CaseWriter + 'a>>> {
        (**self).write_data_file(dictionary)
    }
}

pub trait CaseWriter {
    fn write_case(&mut self, case: Case<Vec<Datum<ByteString>>>) -> anyhow::Result<()>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "driver", rename_all = "snake_case")]
pub enum Config {
    Text(TextConfig),
    Pdf(CairoConfig),
    Html(HtmlConfig),
    Json(JsonConfig),
    Csv(CsvConfig),
    Por(PorConfig),
    Sav(SavConfig),
    Spv(SpvConfig),
}

impl dyn Driver {
    /// Creates a driver for writing to `file`.  If `file` is `None`, then the
    /// driver will write to stdout.  `options` may specify options to pass to
    /// the driver, in TOML format.
    ///
    /// The driver used is the one specified in `options`.  If `options` doesn't
    /// specify a driver, then:
    ///
    /// - If `file` is provided, then it is chosen based on `file`'s extension,
    ///   returning an error if the extension is unknown.
    ///
    /// - If `file` is `None`, then `default_driver` is the driver.
    pub fn from_options<P>(
        file: Option<P>,
        options: &[String],
        default_driver: &str,
    ) -> anyhow::Result<Box<Self>>
    where
        P: AsRef<Path>,
    {
        // Compose initial TOML from the options.
        let mut config = String::new();
        for option in options {
            writeln!(&mut config, "{option}").unwrap();
        }
        let mut config: toml::Table = toml::from_str(&config)?;

        // Insert `file`, if we have one.
        let file = file.as_ref().map(|p| p.as_ref());
        if let Some(file) = file {
            let Some(file) = file.to_str() else {
                bail!("{}: not a valid UTF-8 filename", file.display())
            };
            config.insert(String::from("file"), toml::Value::String(file.into()));
        }

        // Choose a driver.
        if !config.contains_key("driver") {
            let driver = if let Some(file) = file {
                <dyn Driver>::driver_type_from_filename(file).ok_or_else(|| {
                    anyhow!("{}: no default output format for file name", file.display())
                })?
            } else {
                default_driver
            };
            config.insert(String::from("driver"), toml::Value::String(driver.into()));
        }

        Self::new(&Config::deserialize(config)?)
    }

    pub fn new(config: &Config) -> anyhow::Result<Box<Self>> {
        match config {
            Config::Csv(csv_config) => Ok(Box::new(CsvDriver::new(csv_config)?)),
            Config::Html(html_config) => Ok(Box::new(HtmlDriver::new(html_config)?)),
            Config::Json(json_config) => Ok(Box::new(JsonDriver::new(json_config)?)),
            Config::Pdf(cairo_config) => Ok(Box::new(CairoDriver::new(cairo_config)?)),
            Config::Por(por_config) => Ok(Box::new(PorDriver::new(por_config)?)),
            Config::Sav(sav_config) => Ok(Box::new(SavDriver::new(sav_config)?)),
            Config::Spv(spv_config) => Ok(Box::new(SpvDriver::new(spv_config)?)),
            Config::Text(text_config) => Ok(Box::new(TextDriver::new(text_config)?)),
        }
    }

    pub fn driver_type_from_filename(file: impl AsRef<Path>) -> Option<&'static str> {
        match file.as_ref().extension()?.to_str()? {
            "txt" | "text" => Some("text"),
            "pdf" => Some("pdf"),
            "htm" | "html" => Some("html"),
            "csv" => Some("csv"),
            "json" | "ndjson" => Some("json"),
            "spv" => Some("spv"),
            "sav" => Some("sav"),
            "por" => Some("por"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use crate::output::drivers::Config;

    #[test]
    fn toml() {
        let config = r#"driver = "text"
file = "filename.text"
"#;
        let toml: Config = toml::from_str(config).unwrap();
        println!("{}", toml::to_string_pretty(&toml).unwrap());

        #[derive(Serialize)]
        struct Map<'a> {
            file: &'a str,
        }
        println!(
            "{}",
            toml::to_string_pretty(&Map { file: "filename" }).unwrap()
        );
    }
}
