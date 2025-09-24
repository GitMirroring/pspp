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

//! Reading and writing SPV files.
//!
//! This module enables reading and writing SPSS Viewer or `.spv` files, which
//! SPSS 16 and later uses to represent the contents of its output editor.  See also
//! [SPV file format documentation].
//!
//! Use [ReadOptions] to read an SPV file.  Use [Writer] to write an SPV file.
//!
//! [SPV file format documentation]: https://pspp.benpfaff.org/manual/spv/index.html

// Warn about missing docs, but not for items declared with `#[cfg(test)]`.
#![cfg_attr(not(test), warn(missing_docs))]

/// Reading SPV files.
pub mod read;
mod write;

pub use read::{Error, SpvArchive, SpvFile, SpvOutline, html, legacy_bin};
pub use write::Writer;
