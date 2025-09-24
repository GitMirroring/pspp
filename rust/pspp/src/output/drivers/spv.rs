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
    io::{Seek, Write},
    path::PathBuf,
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{
    output::{Item, drivers::Driver, page::PageSetup},
    spv::Writer,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpvConfig {
    /// Output file name.
    pub file: PathBuf,

    /// Page setup.
    pub page_setup: Option<PageSetup>,
}

pub struct SpvDriver<W>
where
    W: Write + Seek,
{
    writer: Writer<W>,
}

impl<W> Driver for SpvDriver<W>
where
    W: Write + Seek + 'static,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::from("spv")
    }

    fn write(&mut self, item: &Arc<Item>) {
        self.writer.write(item).unwrap();
    }

    fn setup(&mut self, page_setup: &PageSetup) -> bool {
        self.writer.set_page_setup(page_setup.clone());
        true
    }

    fn handles_show(&self) -> bool {
        true
    }

    fn handles_groups(&self) -> bool {
        true
    }
}

impl SpvDriver<File> {
    pub fn new(config: &SpvConfig) -> std::io::Result<Self> {
        let mut writer = Writer::for_writer(File::create(&config.file)?).unwrap();
        if let Some(page_setup) = &config.page_setup {
            writer = writer.with_page_setup(page_setup.clone());
        }
        Ok(Self { writer })
    }
}

impl<W> SpvDriver<W>
where
    W: Write + Seek,
{
    pub fn for_writer(writer: W) -> Self {
        Self {
            writer: Writer::for_writer(writer).unwrap(),
        }
    }
}
