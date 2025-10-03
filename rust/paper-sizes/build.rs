use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

fn main() {
    build_bindings();
    build_paperspecs();
}

fn build_bindings() {
    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}

fn build_paperspecs() {
    let output_name = PathBuf::from(env::var("OUT_DIR").unwrap()).join("paperspecs.rs");
    let mut output = File::create(&output_name).expect("Failed to create output file");

    println!("cargo:rerun-if-changed=paperspecs");
    let reader =
        BufReader::new(File::open("paperspecs").expect("Unable to open `paperspecs` file"));
    let mut ids = HashSet::new();
    let mut fallbacks = Vec::new();
    for line in reader
        .lines()
        .map(|result| result.expect("Error reading `paperspecs` file"))
    {
        let mut splitter = line.split(',');
        let name = splitter.next().unwrap();
        let raw_width = splitter.next().unwrap();
        let raw_height = splitter.next().unwrap();
        let raw_unit = splitter.next().unwrap();

        // Turn name into Rust identifier.
        let mut id = name.to_ascii_uppercase().replace(' ', "_");
        if !id.chars().next().unwrap().is_alphabetic() {
            id.insert_str(0, "PAPER_");
        }
        let alternate = if !ids.insert(id.clone()) {
            id.push_str("_ISO");
            true
        } else {
            false
        };
        fallbacks.push(id.clone());

        // Make width and height valid Rust floating-point numbers.
        fn to_float(s: &str) -> String {
            let mut s = String::from(s);
            if !s.contains('.') {
                s.push_str(".0");
            }
            s
        }
        let width = to_float(raw_width);
        let height = to_float(raw_height);

        // Turn unit into Rust identifier.
        let unit = match raw_unit {
            "mm" => "Millimeter",
            "in" => "Inch",
            "pt" => "Point",
            _ => panic!("Unknown unit {raw_unit}."),
        };

        writeln!(
            &mut output,
            "/// {name} paper size{} ({raw_width} x {raw_height} {raw_unit}).
pub static {id}: PaperSpec = PaperSpec {{
  name: Cow::Borrowed(\"{name}\"),
  size: PaperSize {{
     width: {width},
     height: {height},
     unit: Unit::{unit}
  }},
}};
",
            if alternate { " in millimeters" } else { "" }
        )
        .unwrap();
    }

    writeln!(
        &mut output,
        "/// Paper specifications from the standard `paperspecs` file.
pub static STANDARD_PAPERSPECS: [&PaperSpec; {}] = [",
        fallbacks.len()
    )
    .unwrap();
    for id in fallbacks {
        writeln!(&mut output, "    &{},", id).unwrap();
    }
    writeln!(&mut output, "];").unwrap();
}
