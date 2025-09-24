use std::{
    fs::File,
    io::{BufRead, BufReader, Seek},
    path::Path,
};

use crate::{
    output::{
        Text,
        drivers::text::{Emphasis, TextRenderer},
        pivot::tests::assert_lines_eq,
    },
    spv::SpvArchive,
};

/// Tests for compatible handling of invalid HTML entity syntax.
#[test]
fn text1() {
    test_raw_spvfile("text1", None);
}

/// Checks that reordering categories works properly.
#[test]
fn light1() {
    test_raw_spvfile("light1", None);
}

/// Checks that dimensions are ordered properly and that the title reflects the
/// user's changes.
#[test]
fn light2() {
    test_raw_spvfile("light2", None);
}

/// Checks against regression for a bug reading borders.
#[test]
fn light3() {
    test_raw_spvfile("light3", None);
}

/// Further check for dimensions and axes are ordered properly.
#[test]
fn light4() {
    test_raw_spvfile("light4", None);
}

/// Test `[A:B:C]` templates.
#[test]
fn light5() {
    test_raw_spvfile("light5", None);
}

/// Test categories to be deleted due to negative `leaf-index`.
#[test]
fn light6() {
    test_raw_spvfile("light6", None);
}

/// Test current layer selection.
#[test]
fn light7() {
    test_raw_spvfile("light7", None);
}

/// Test a different current layer selection.
#[test]
fn light8() {
    test_raw_spvfile("light8", None);
}

#[test]
fn legacy1() {
    test_raw_spvfile("legacy1", None);
}

#[test]
fn legacy2() {
    test_raw_spvfile("legacy2", None);
}

#[test]
fn legacy3() {
    test_raw_spvfile("legacy3", None);
}

#[test]
fn legacy4() {
    test_raw_spvfile("legacy4", None);
}

/// Layers.
#[test]
fn legacy5() {
    test_raw_spvfile("legacy5", None);
}

/// Layer, with a particular layer selected.
#[test]
fn legacy6() {
    test_raw_spvfile("legacy6", None);
}

/// Regression test for `<setFormat reset="true">`.
#[test]
fn legacy7() {
    test_raw_spvfile("legacy7", None);
}

/// Checks for `Dimension::hide_all_labels`.
#[test]
fn legacy8() {
    test_raw_spvfile("legacy8", None);
}

/// Checks for caption defined as a footnote label, and for footnotes in layer
/// values.
#[test]
fn legacy9() {
    test_raw_spvfile("legacy9", None);
}

/// Checks for footnotes in dimension labels.
#[test]
fn legacy10() {
    test_raw_spvfile("legacy10", None);
}

/// Checks for footnotes on data cells added via XML rather than `cellFootnotes`
/// series.
#[test]
fn legacy11() {
    test_raw_spvfile("legacy11", None);
}

/// Checks that footnotes on data cells added via the `cellFootnotes` series
/// can't be deleted via `setFormat`.
#[test]
fn legacy12() {
    test_raw_spvfile("legacy12", None);
}

/// Checks that we support multiple `<setFormat>` elements within a single
/// `<setCellProperties>`.
#[test]
fn legacy13() {
    test_raw_spvfile("legacy13", None);
}

/// Check for correct defaults in XML looks.
#[test]
fn legacy14() {
    test_raw_spvfile("legacy14", None);
}

/// Checks that categories are ordered correctly when the first row has some
/// missing cells (in this case, "Beta" lacks a value in the first row).
#[test]
fn legacy15() {
    test_raw_spvfile("legacy15", None);
}

/// Subscript support.
#[test]
fn legacy16() {
    test_raw_spvfile("legacy16", None);
}

/// Support styling cells with colored backgrounds.
#[test]
fn legacy17() {
    test_raw_spvfile("legacy17", Some(Emphasis::Ansi));
}

fn test_raw_spvfile(name: &str, emphasis: Option<Emphasis>) {
    let input_filename = Path::new("src/spv/testdata")
        .join(name)
        .with_extension("spv");
    let spvfile = BufReader::new(File::open(&input_filename).unwrap());
    let expected_filename = input_filename.with_extension("expected");
    let expected = String::from_utf8(std::fs::read(&expected_filename).unwrap()).unwrap();
    test_spvfile(spvfile, &expected, &expected_filename, emphasis);
}

fn test_spvfile<R>(spvfile: R, expected: &str, expected_filename: &Path, emphasis: Option<Emphasis>)
where
    R: BufRead + Seek + 'static,
{
    let mut warnings = Vec::new();
    let result = match SpvArchive::open_reader(spvfile, None) {
        Ok(mut archive) => archive.read(|w| warnings.push(w)),
        Err(error) => Err(error),
    };

    let output = match result {
        Ok(spv_file) => {
            let (items, _page_setup /*XXX*/) = spv_file.into_contents();

            let mut output = Vec::new();
            /* XXX
            output.extend(
                warnings
                    .into_iter()
                    .map(|warning| Item::from(Text::new_log(warning.to_string()))),
            );*/
            output.extend(items);
            output.into_iter().collect()
        }
        Err(error) => Text::new_log(error.to_string()).into_item(),
    };

    let mut renderer = TextRenderer::default().with_emphasis(emphasis);
    let mut actual = String::new();
    renderer.render(&output, &mut actual).unwrap();
    if expected != actual {
        if std::env::var("PSPP_REFRESH_EXPECTED").is_ok() {
            std::fs::write(expected_filename, actual).unwrap();
            panic!("{}: refreshed output", expected_filename.display());
        } else {
            eprintln!("note: rerun with PSPP_REFRESH_EXPECTED=1 to refresh expected output");
        }
    }
    assert_lines_eq(&expected, expected_filename.display(), &actual, "actual");
}
