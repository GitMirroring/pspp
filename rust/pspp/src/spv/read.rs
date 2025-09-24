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
#![allow(dead_code)]
use std::{
    fmt::Display,
    fs::File,
    io::{BufReader, Read, Seek},
    path::Path,
};

use displaydoc::Display;
use serde::Deserialize;
use zip::{ZipArchive, result::ZipError};

use crate::{
    crypto::EncryptedReader,
    output::{Item, page::PageSetup},
    spv::{
        legacy_bin::LegacyBinWarning,
        read::{
            legacy_xml::LegacyXmlWarning,
            light::LightWarning,
            structure::{OutlineItem, StructureMember},
        },
    },
};

mod css;
pub mod html;
pub mod legacy_bin;
#[allow(missing_docs)]
pub mod legacy_xml;
mod light;
pub mod structure;
#[cfg(test)]
mod tests;

/// A warning encountered reading an SPV file.
#[derive(Clone, Debug)]
pub struct Warning {
    /// The name of the .zip file member inside the system file.
    pub member: String,
    /// Detailed warning message.
    pub details: WarningDetails,
}

impl Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Warning reading member {:?}: {}",
            &self.member, &self.details
        )
    }
}

/// Details of a [Warning].
#[derive(Clone, Debug, thiserror::Error, Display)]
pub enum WarningDetails {
    /// {0}
    LightWarning(LightWarning),

    /// {0}
    LegacyBinWarning(LegacyBinWarning),

    /// {0}
    LegacyXmlWarning(LegacyXmlWarning),

    /// Unknown page orientation {0:?}.
    UnknownOrientation(String),
}

/// Both [Read] and [Seek] together.
pub trait ReadSeek: Read + Seek {}
impl<T> ReadSeek for T where T: Read + Seek {}

/// A ZIP archive that contains an SPV file.
pub struct SpvArchive<R>(pub ZipArchive<R>);

impl SpvArchive<Box<dyn ReadSeek>> {
    /// Opens the file at `path`.
    pub fn open_file<P>(path: P, password: Option<&str>) -> Result<Self, Error>
    where
        P: AsRef<Path>,
    {
        Self::open_reader(File::open(path)?, password)
    }

    /// Opens the provided Zip `archive`.
    ///
    /// Any password provided for reading the file is unused, because if one was
    /// needed then it must have already been used to open the archive.
    pub fn open_reader<R>(reader: R, password: Option<&str>) -> Result<Self, Error>
    where
        R: Read + Seek + 'static,
    {
        let reader = if let Some(password) = password {
            Box::new(EncryptedReader::open(reader, password)?)
        } else {
            Box::new(reader) as Box<dyn ReadSeek>
        };
        let mut archive = ZipArchive::new(reader).map_err(|error| match error {
            ZipError::InvalidArchive(_) => Error::NotSpv,
            other => other.into(),
        })?;
        let mut file = archive
            .by_name("META-INF/MANIFEST.MF")
            .map_err(|_| Error::NotSpv)?;
        let mut string = String::new();
        file.read_to_string(&mut string)?;
        if string.trim() != "allowPivoting=true" {
            return Err(Error::NotSpv);
        }
        drop(file);

        Ok(Self(archive))
    }
}

impl<R> SpvArchive<R>
where
    R: Read + Seek,
{
    /// Reads and returns an outline of the contents of the SPV file.
    ///
    /// Most callers want to use [read](Self::read), instead, to read the full
    /// contents of the SPV file.
    pub fn read_outline<F>(&mut self, mut warn: F) -> Result<SpvOutline, Error>
    where
        F: FnMut(Warning),
    {
        // Read all the items.
        let mut items = Vec::new();
        let mut page_setup = None;
        for i in 0..self.0.len() {
            let name = String::from(self.0.name_for_index(i).unwrap());
            if name.starts_with("outputViewer") && name.ends_with(".xml") {
                let member = BufReader::new(self.0.by_index(i)?);
                let mut member = StructureMember::read(member, &name, &mut warn)?;
                page_setup = page_setup.or(member.page_setup);
                items.append(&mut member.items);
            }
        }
        Ok(SpvOutline { page_setup, items })
    }

    /// Reads and returns the whole SPV file contents.
    pub fn read<F>(&mut self, mut warn: F) -> Result<SpvFile, Error>
    where
        F: FnMut(Warning),
    {
        self.read_outline(&mut warn)?.read_items(self, &mut warn)
    }
}

/// An outline of an SPV file.
///
/// Reading the outline only requires reading some of the SPV file.  It provides
/// a "table of contents" view of the SPV, without reading the details of tables
/// and graphs.
#[derive(Clone, Debug)]
pub struct SpvOutline {
    /// Optional page setup, from the first structure member.
    pub page_setup: Option<PageSetup>,

    /// The table of contents.
    pub items: Vec<OutlineItem>,
}

impl SpvOutline {
    fn read_items<F, R>(self, archive: &mut SpvArchive<R>, warn: &mut F) -> Result<SpvFile, Error>
    where
        R: Read + Seek,
        F: FnMut(Warning),
    {
        Ok(SpvFile {
            page_setup: self.page_setup,
            items: self
                .items
                .into_iter()
                .map(|member| member.read_item(&mut archive.0, warn))
                .collect(),
        })
    }
}

/// A SPSS viewer (SPV) file read with [ReadOptions].
pub struct SpvFile {
    /// SPV file contents.
    pub items: Vec<Item>,

    /// The page setup in the SPV file, if any.
    pub page_setup: Option<PageSetup>,
}

impl SpvFile {
    /// Returns the contents of the `SpvFile`.
    pub fn into_contents(self) -> (Vec<Item>, Option<PageSetup>) {
        (self.items, self.page_setup)
    }

    /// Returns just the [Item]s.
    pub fn into_items(self) -> Vec<Item> {
        self.items
    }
}

/// An error reading an SPV file.
///
/// Returned by [ReadOptions::open_file] and [ReadOptions::open_reader].
#[derive(Debug, Display, thiserror::Error)]
pub enum Error {
    /// Not an SPV file.
    NotSpv,

    /// {0}
    EncryptionError(#[from] crate::crypto::Error),

    /// {0}
    ZipError(#[from] ZipError),

    /// {0}
    IoError(#[from] std::io::Error),

    /// {0}
    DeError(#[from] quick_xml::DeError),

    /// {0}
    BinrwError(#[from] binrw::Error),

    /// {0}
    CairoError(#[from] cairo::IoError),

    /// Error parsing {member_name}: {error}
    DeserializeError {
        /// The name of the file inside the ZIP file that caused the error.
        member_name: String,
        /// Underlying error.
        error: serde_path_to_error::Error<quick_xml::DeError>,
    },

    /// Legacy table missing `graph` element.
    LegacyMissingGraph,

    /// Graphs not yet implemented.
    GraphTodo,

    /// Models not yet implemented.
    ModelTodo,

    /// Trees not yet implemented.
    TreeTodo,
}

#[derive(Copy, Clone, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum TableType {
    Table,
    Note,
    Warning,
}
