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

use anyhow::Result;
use clap::{Parser, Subcommand};
use encoding_rs::Encoding;
use thiserror::Error as ThisError;

use convert::Convert;
use decrypt::Decrypt;
use identify::Identify;
use show::Show;
use show_pc::ShowPc;
use show_por::ShowPor;
use show_spv::ShowSpv;

mod convert;
mod decrypt;
mod identify;
mod show;
mod show_pc;
mod show_por;
mod show_spv;

/// PSPP, a program for statistical analysis of sampled data.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Clone, Debug)]
pub enum Command {
    Convert(Convert),
    Decrypt(Decrypt),
    Identify(Identify),
    Show(Show),
    ShowPor(ShowPor),
    ShowPc(ShowPc),
    ShowSpv(ShowSpv),
}

impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Command::Convert(convert) => convert.run(),
            Command::Decrypt(decrypt) => decrypt.run(),
            Command::Identify(identify) => identify.run(),
            Command::Show(show) => show.run(),
            Command::ShowPor(show_por) => show_por.run(),
            Command::ShowPc(show_pc) => show_pc.run(),
            Command::ShowSpv(show_spv) => show_spv.run(),
        }
    }
}

#[derive(ThisError, Debug)]
#[error("{0}: unknown encoding")]
struct UnknownEncodingError(String);

fn parse_encoding(arg: &str) -> Result<&'static Encoding, UnknownEncodingError> {
    match Encoding::for_label_no_replacement(arg.as_bytes()) {
        Some(encoding) => Ok(encoding),
        None => Err(UnknownEncodingError(arg.to_string())),
    }
}
