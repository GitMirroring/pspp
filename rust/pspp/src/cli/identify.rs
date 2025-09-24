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

use anyhow::Result;
use clap::Args;
use pspp::file::FileType;
use std::path::PathBuf;

/// Identify the type of a file.
#[derive(Args, Clone, Debug)]
pub struct Identify {
    /// File to identify.
    file: PathBuf,
}

impl Identify {
    pub fn run(self) -> Result<()> {
        match FileType::from_file(&self.file)? {
            None => println!("unknown"),
            Some(file_type) => {
                print!("{}", file_type.as_extension());
                if file_type.is_encrypted() {
                    print!(" (encrypted)");
                }
                if !file_type.is_confident() {
                    print!(" (low confidence)");
                }
                println!();
            }
        }
        Ok(())
    }
}
