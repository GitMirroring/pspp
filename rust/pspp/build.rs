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

use anyhow::{Result as AnyResult, anyhow};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    env::var_os,
    ffi::OsStr,
    fmt::Write as _,
    fs::{File, read_to_string},
    io::{Error as IoError, Write},
    path::{Path, PathBuf},
};
use zip::ZipArchive;

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd)]
enum Source {
    Codepage,
    Ibm,
    Windows,
}

// Code page number.
type CodepageNumber = usize;

fn process_converter<'a>(
    fields: &[&'a str],
    codepages: &mut BTreeMap<CodepageNumber, BTreeMap<Source, Vec<&'a str>>>,
) {
    if fields.is_empty() || fields[0] == "{" {
        return;
    }

    let mut cps: BTreeMap<Source, CodepageNumber> = BTreeMap::new();
    let mut iana = VecDeque::new();
    let mut other = VecDeque::new();

    let mut iter = fields.iter().peekable();
    while let Some(&name) = iter.next() {
        if iter.next_if(|&&s| s == "{").is_some() {
            let mut standards = HashSet::new();
            loop {
                let &standard = iter.next().expect("missing `}` in list of standards");
                if standard == "}" {
                    break;
                }
                standards.insert(standard);
            }

            if standards.contains("IANA*") {
                iana.push_front(name);
            } else if standards.contains("IANA") {
                iana.push_back(name);
            } else if standards.iter().any(|&s| s.ends_with('*')) {
                other.push_front(name);
            } else {
                other.push_back(name);
            }
        } else {
            // Untagged names are completely nonstandard.
            continue;
        }

        if let Some(number) = name.strip_prefix("cp") {
            if let Ok(number) = number.parse::<CodepageNumber>() {
                cps.insert(Source::Codepage, number);
            }
        }

        if let Some(number) = name.strip_prefix("windows-") {
            if let Ok(number) = number.parse::<CodepageNumber>() {
                cps.insert(Source::Windows, number);
            }
        }

        if let Some(number) = name.strip_prefix("ibm-") {
            if let Ok(number) = number.parse::<CodepageNumber>() {
                cps.insert(Source::Ibm, number);
            }
        }
    }

    // If there are no tagged names then this is completely nonstandard.
    if iana.is_empty() && other.is_empty() {
        return;
    }

    let all: Vec<&str> = iana.into_iter().chain(other).collect();
    for (source, number) in cps {
        codepages
            .entry(number)
            .or_default()
            .insert(source, all.clone());
    }
}

fn write_output(
    codepages: &BTreeMap<CodepageNumber, BTreeMap<Source, Vec<&str>>>,
    file_name: &PathBuf,
) -> Result<(), IoError> {
    let mut file = File::create(file_name)?;

    file.write_all(
        "\
use std::collections::HashMap;

static CODEPAGE_NUMBER_TO_NAME: LazyLock<HashMap<i32, &'static str>> = LazyLock::new(|| {
    let mut map = HashMap::new();
"
        .as_bytes(),
    )?;

    for (&cpnumber, value) in codepages.iter() {
        let source = value.keys().max().unwrap();
        let name = value[source][0];
        writeln!(file, "    map.insert({cpnumber}, \"{name}\");")?;
    }
    file.write_all(
        "        map
});

static CODEPAGE_NAME_TO_NUMBER: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(|| {
    let mut map = HashMap::new();
"
        .as_bytes(),
    )?;

    let mut names: BTreeMap<String, BTreeMap<Source, Vec<CodepageNumber>>> = BTreeMap::new();
    for (&cpnumber, value) in codepages.iter() {
        for (&source, value2) in value.iter() {
            for name in value2.iter().map(|name| name.to_ascii_lowercase()) {
                names
                    .entry(name)
                    .or_default()
                    .entry(source)
                    .or_default()
                    .push(cpnumber);
            }
        }
    }

    for (name, value) in names.iter() {
        for (_source, numbers) in value.iter().rev().take(1) {
            writeln!(file, "        map.insert(\"{name}\", {});", numbers[0])?;
        }
    }
    file.write_all(
        "        map
});
"
        .as_bytes(),
    )?;

    Ok(())
}

fn build_encodings() -> AnyResult<()> {
    let input_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("convrtrs.txt");
    println!("cargo:rerun-if-changed={}", input_file.display());
    let input = read_to_string(&input_file)
        .map_err(|e| anyhow!("{}: read failed ({e})", input_file.display()))?;

    let mut codepages: BTreeMap<CodepageNumber, BTreeMap<Source, Vec<&str>>> = BTreeMap::new();
    let mut converter: Vec<&str> = Vec::new();
    for line in input.lines() {
        let line = line
            .find('#')
            .map(|position| &line[..position])
            .unwrap_or(line)
            .trim_end();
        if !line.starts_with([' ', '\t']) {
            process_converter(&converter, &mut codepages);
            converter.clear();
        }
        converter.extend(line.split_whitespace());
    }
    process_converter(&converter, &mut codepages);

    for (codepage, source, name) in [
        (20932, Source::Codepage, "EUC-JP"),
        (50220, Source::Codepage, "ISO-2022-JP"),
        (28600, Source::Windows, "ISO-8859-10"),
        (28604, Source::Windows, "ISO-8859-14"),
        (28606, Source::Windows, "ISO-8859-16"),
        (99998, Source::Codepage, "replacement"),
        (99999, Source::Codepage, "x-user-defined"),
    ] {
        assert!(
            codepages
                .insert(codepage, [(source, vec![name])].into_iter().collect())
                .is_none()
        );
    }

    let output_file_name = Path::new(&var_os("OUT_DIR").unwrap()).join("encodings.rs");

    write_output(&codepages, &output_file_name)
        .map_err(|e| anyhow!("{}: write failed ({e})", output_file_name.display()))?;
    Ok(())
}

/// Read `cldr-json-full.zip` and write `src/format/decimals.rs`.
fn builld_decimals() -> AnyResult<()> {
    let cldr_path = Path::new("cldr-json-full.zip");
    if !cldr_path.try_exists()? {
        // Don't try to regenerate `decimals.rs`, since we don't have the CLDR.
        return Ok(());
    }
    println!("cargo:rerun-if-changed={}", cldr_path.display());

    let mut archive = ZipArchive::new(File::open(cldr_path)?)?;
    let mut decimals = BTreeMap::new();
    for i in 0..archive.len() {
        let name = Path::new(archive.name_for_index(i).unwrap());
        if name.file_name() == Some(OsStr::new("numbers.json")) {
            let file = archive.by_index(i)?;
            let json: Value = serde_json::from_reader(file).expect("should be JSON");
            if let Some(main) = json.get("main")
                && let Some(langs) = main.as_object()
                && let Some((lang, details)) = langs.iter().next()
                && let Some(numbers) = details.get("numbers")
                && let Some(latin) = numbers.get("symbols-numberSystem-latn")
                && let Some(decimal) = latin.get("decimal")
                && let Some(decimal) = decimal.as_str()
                && let Some(decimal) = decimal.chars().next()
                && (decimal == '.' || decimal == ',')
            {
                decimals.insert(lang.clone(), decimal);
            }
        }
    }
    let decimals = decimals.iter().filter(|(name, decimal)| {
        if let Some((stem, _)) = name.split_once('-')
            && decimals.get(stem) == Some(*decimal)
        {
            false
        } else {
            true
        }
    });

    let mut contents = String::new();
    write!(
        &mut contents,
        "//! Autogenerated, do not edit by hand!
//!
//! The code in this file is generated by `build.rs`.  It is unconventional
//! to check generated files into a repository, but it is done in this case
//! because the source data is large and the generated code rarely needs to
//! change.
//!
//! To regenerate this code, download a CLDR JSON release from
//! <https://cldr.unicode.org/index/downloads>, rename it as
//! `cldr-json-full.zip` in the same directory as `build.rs`,
//! and touch `build.rs` to force a rebuild.
use std::{{collections::HashMap, sync::LazyLock}};
use crate::format::Decimal;

/// Map from language to decimal point.
pub static LANG_TO_DECIMAL: LazyLock<HashMap<&'static str, Decimal>> = LazyLock::new(|| {{
    let mut map = HashMap::new();
"
    )
    .unwrap();
    for (lang, decimal) in decimals {
        let decimal = if *decimal == ',' { "Comma" } else { "Dot" };
        writeln!(
            &mut contents,
            "    map.insert({lang:?}, Decimal::{decimal});"
        )
        .unwrap();
    }
    write!(
        &mut contents,
        "    map
}});
"
    )
    .unwrap();

    let output_file_name = Path::new("src/format/decimals.rs");
    if !output_file_name.try_exists()? || contents.as_bytes() != &std::fs::read(&output_file_name)?
    {
        std::fs::write(&output_file_name, &contents)?;
    }

    Ok(())
}

fn main() -> AnyResult<()> {
    println!("cargo:rerun-if-changed=build.rs");

    build_encodings()?;
    builld_decimals()?;

    Ok(())
}
