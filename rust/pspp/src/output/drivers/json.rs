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
    fs::File,
    io::{BufWriter, Write, stdout},
    path::PathBuf,
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use super::{Driver, Item};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonConfig {
    file: Option<PathBuf>,

    /// If false (the default), each output item is exactly one line of JSON, in
    /// [newline-delimited JSON] format.
    ///
    /// If true, each output item is pretty-printed as multiple lines.
    ///
    /// [newline-delimited JSON]: https://github.com/ndjson/ndjson-spec
    pretty: Option<bool>,

    /// If false (the default), output items that support a specialized
    /// serialization format that accurately represents their semantics will be
    /// output that way.  (Output items that don't will be output as pivot
    /// tables.)
    ///
    /// If true, output items always output as pivot tables and other SPSS
    /// output items.  (Some output items can't be output this way.)
    tables: Option<bool>,
}

pub struct JsonDriver {
    file: Box<dyn Write>,
    pretty: bool,
    tables: bool,
}

impl JsonDriver {
    pub fn new(config: &JsonConfig) -> std::io::Result<Self> {
        Ok(Self {
            file: match &config.file {
                Some(file) => Box::new(BufWriter::new(File::create(file)?)),
                None => Box::new(stdout()),
            },
            pretty: config.pretty.unwrap_or_else(|| {
                !config
                    .file
                    .as_ref()
                    .is_some_and(|file| file.ends_with(".ndjson"))
            }),
            tables: config.tables.unwrap_or_default(),
        })
    }

    fn write_json<T>(&mut self, item: &T) -> std::io::Result<()>
    where
        T: Serialize + ?Sized,
    {
        if self.pretty {
            serde_json::to_writer_pretty(&mut self.file, item)?;
        } else {
            serde_json::to_writer(&mut self.file, item)?;
        }
        writeln!(&mut self.file)
    }
}

impl Driver for JsonDriver {
    fn name(&self) -> Cow<'static, str> {
        Cow::from("json")
    }

    fn write(&mut self, item: &Arc<Item>) {
        self.write_json(item).unwrap(); // XXX handle errors.
    }

    fn can_serialize(&self) -> bool {
        !self.tables
    }

    fn serialize(&mut self, item: &dyn erased_serde::Serialize) {
        assert!(!self.tables);
        self.write_json(item).unwrap(); // XXX handle errors
    }

    fn flush(&mut self) {
        let _ = self.file.flush();
    }
}
