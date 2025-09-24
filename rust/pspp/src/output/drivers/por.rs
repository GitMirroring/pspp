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

use std::{borrow::Cow, fs::File, io::BufWriter, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::{
    data::{ByteString, Case, Datum},
    dictionary::Dictionary,
    por::{WriteOptions, Writer},
};

use super::{CaseWriter, Driver, Item};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PorConfig {
    file: PathBuf,
}

pub struct PorDriver {
    file: PathBuf,
}

impl PorDriver {
    pub fn new(config: &PorConfig) -> std::io::Result<Self> {
        Ok(Self {
            file: config.file.clone(),
        })
    }
}

impl Driver for PorDriver {
    fn name(&self) -> Cow<'static, str> {
        Cow::from("por")
    }

    fn write(&mut self, _item: &Arc<Item>) {
        todo!()
    }

    fn can_write_data_file(&self) -> bool {
        true
    }

    fn write_data_file<'a>(
        &'a mut self,
        dictionary: &'a Dictionary,
    ) -> anyhow::Result<Option<Box<dyn CaseWriter + 'a>>> {
        Ok(Some(Box::new(PorDriverCaseWriter {
            writer: WriteOptions::new().write_file(&dictionary, &self.file)?,
        })))
    }
}

struct PorDriverCaseWriter {
    writer: Writer<BufWriter<File>>,
}

impl CaseWriter for PorDriverCaseWriter {
    fn write_case(&mut self, case: Case<Vec<Datum<ByteString>>>) -> anyhow::Result<()> {
        self.writer.write_case(case)?;
        Ok(())
    }
}
