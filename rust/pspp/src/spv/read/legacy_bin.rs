//! Legacy binary data.
use std::io::{Read, Seek, SeekFrom};

use binrw::{BinRead, BinResult, binread};
use displaydoc::Display;
use encoding_rs::UTF_8;
use indexmap::IndexMap;

use crate::{
    data::Datum,
    spv::read::light::{U32String, parse_vec},
};

/// A warning decoding a legacy binary member.
#[derive(Clone, Debug, Display, thiserror::Error)]
pub enum LegacyBinWarning {
    /// String map refers to unknown source {0:?}.
    UnknownSource(String),
    /// String map for source {source_name:?} refers to unknown variable {variable:?}.
    UnknownVariable {
        /// Source name.
        source_name: String,
        /// Variable name.
        variable: String,
    },
    /// Datum mapping {datum_idx} for variable {variable:?} in source {source_name:?} has label index {label_idx} but there are only {n_labels} labels.
    OutOfRangeLabelIdx {
        /// Source name.
        source_name: String,
        /// Variable name.
        variable: String,
        /// Index into variable's datum map.
        datum_idx: usize,
        /// Too-large index into labels.
        label_idx: usize,
        /// Number of labels.
        n_labels: usize,
    },
    /// Datum mapping {datum_idx} for variable {variable:?} in source {source_name:?} has value index {value_idx} but the variable has only {n_values} values.
    OutOfRangeValueIdx {
        /// Source name.
        source_name: String,
        /// Variable name.
        variable: String,
        /// Index into variable's datum map.
        datum_idx: usize,
        /// Too-large index into values.
        value_idx: usize,
        /// Number of values in `variable`.
        n_values: usize,
    },
}

/// Legacy binary data.
#[binread]
#[br(little)]
#[derive(Debug)]
pub struct LegacyBin {
    #[br(magic(0u8), temp)]
    version: Version,
    #[br(temp)]
    n_sources: u16,
    #[br(temp)]
    _member_size: u32,
    #[br(count(n_sources), args { inner: (version,) })]
    metadata: Vec<Metadata>,
    #[br(parse_with(parse_data), args(metadata.as_slice()))]
    data: Vec<Data>,
    #[br(parse_with(parse_strings))]
    strings: Option<Strings>,
}

impl LegacyBin {
    /// Decodes legacy binary data into a map from a series name to a map of
    /// variables, which in turn contains a vector of [Datum]s.
    pub fn decode(
        &self,
        warn: &mut dyn FnMut(LegacyBinWarning),
    ) -> IndexMap<String, IndexMap<String, Vec<Datum<String>>>> {
        let mut sources = IndexMap::new();
        for (metadata, data) in self.metadata.iter().zip(&self.data) {
            let mut variables = IndexMap::new();
            for variable in &data.variables {
                variables.insert(
                    variable.variable_name.clone(),
                    variable
                        .values
                        .iter()
                        .map(|value| Datum::Number((*value != f64::MIN).then_some(*value)))
                        .collect::<Vec<_>>(),
                );
            }
            sources.insert(metadata.source_name.clone(), variables);
        }
        if let Some(strings) = &self.strings {
            for map in &strings.source_maps {
                let Some(source) = sources.get_mut(&map.source_name) else {
                    warn(LegacyBinWarning::UnknownSource(map.source_name.clone()));
                    continue;
                };
                for var_map in &map.variable_maps {
                    let Some(variable) = source.get_mut(&var_map.variable_name) else {
                        warn(LegacyBinWarning::UnknownVariable {
                            source_name: map.source_name.clone(),
                            variable: var_map.variable_name.clone(),
                        });
                        continue;
                    };
                    for (datum_idx, datum_map) in var_map.datum_maps.iter().enumerate() {
                        let Some(label) = strings.labels.get(datum_map.label_idx) else {
                            warn(LegacyBinWarning::OutOfRangeLabelIdx {
                                source_name: map.source_name.clone(),
                                variable: var_map.variable_name.clone(),
                                datum_idx,
                                label_idx: datum_map.label_idx,
                                n_labels: strings.labels.len(),
                            });
                            continue;
                        };
                        let Some(value) = variable.get_mut(datum_map.value_idx) else {
                            warn(LegacyBinWarning::OutOfRangeValueIdx {
                                source_name: map.source_name.clone(),
                                variable: var_map.variable_name.clone(),
                                datum_idx,
                                value_idx: datum_map.value_idx,
                                n_values: variable.len(),
                            });
                            continue;
                        };
                        *value = Datum::String(label.label.clone());
                    }
                }
            }
        }
        sources
    }
}

#[binread]
#[br(little)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Version {
    #[br(magic = 0xafu8)]
    Vaf,
    #[br(magic = 0xb0u8)]
    Vb0,
}

#[binread]
#[br(little, import(version: Version))]
#[derive(Debug)]
struct Metadata {
    n_values: u32,
    n_variables: u32,
    data_offset: u32,
    #[br(parse_with(parse_fixed_utf8_string), args(if version == Version::Vaf { 28 } else { 64 }))]
    source_name: String,
    #[br(if(version == Version::Vb0), temp)]
    _x: u32,
}

#[derive(Debug)]
struct Data {
    variables: Vec<Variable>,
}

#[binrw::parser(reader, endian)]
fn parse_data(metadata: &[Metadata]) -> BinResult<Vec<Data>> {
    let mut data = Vec::with_capacity(metadata.len());
    for metadata in metadata {
        reader.seek(SeekFrom::Start(metadata.data_offset as u64))?;
        let mut variables = Vec::with_capacity(metadata.n_variables as usize);
        for _ in 0..metadata.n_variables {
            variables.push(Variable::read_options(
                reader,
                endian,
                (metadata.n_values,),
            )?);
        }
        data.push(Data { variables });
    }
    Ok(data)
}

impl BinRead for Data {
    type Args<'a> = &'a [Metadata];

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        metadata: Self::Args<'_>,
    ) -> binrw::BinResult<Self> {
        let mut variables = Vec::with_capacity(metadata.len());
        for metadata in metadata {
            reader.seek(SeekFrom::Start(metadata.data_offset as u64))?;
            variables.push(Variable::read_options(
                reader,
                endian,
                (metadata.n_values,),
            )?);
        }
        Ok(Self { variables })
    }
}

#[binread]
#[br(little, import(n_values: u32))]
#[derive(Debug)]
struct Variable {
    #[br(parse_with(parse_fixed_utf8_string), args(288))]
    variable_name: String,
    #[br(count(n_values))]
    values: Vec<f64>,
}

#[binrw::parser(reader, endian)]
fn parse_strings() -> BinResult<Option<Strings>> {
    let position = reader.stream_position()?;
    let length = reader.seek(SeekFrom::End(0))?;
    if position != length {
        reader.seek(SeekFrom::Start(position))?;
        Ok(Some(Strings::read_options(reader, endian, ())?))
    } else {
        Ok(None)
    }
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct Strings {
    #[br(parse_with(parse_vec))]
    source_maps: Vec<SourceMap>,
    #[br(parse_with(parse_vec))]
    labels: Vec<Label>,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct SourceMap {
    #[br(parse_with(parse_utf8_string))]
    source_name: String,
    #[br(parse_with(parse_vec))]
    variable_maps: Vec<VariableMap>,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct VariableMap {
    #[br(parse_with(parse_utf8_string))]
    variable_name: String,
    #[br(parse_with(parse_vec))]
    datum_maps: Vec<DatumMap>,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct DatumMap {
    #[br(map(|x: u32| x as usize))]
    value_idx: usize,
    #[br(map(|x: u32| x as usize))]
    label_idx: usize,
}

#[binread]
#[br(little)]
#[derive(Debug)]
struct Label {
    #[br(temp)]
    _frequency: u32,
    #[br(parse_with(parse_utf8_string))]
    label: String,
}

/// Parses a UTF-8 string preceded by a 32-bit length.
#[binrw::parser(reader, endian)]
fn parse_utf8_string() -> BinResult<String> {
    Ok(U32String::read_options(reader, endian, ())?.decode(UTF_8))
}

/// Parses a UTF-8 string that is exactly `n` bytes long and whose contents end
/// at the first null byte.
#[binrw::parser(reader)]
fn parse_fixed_utf8_string(n: usize) -> BinResult<String> {
    let mut buf = vec![0; n];
    reader.read_exact(&mut buf)?;
    if let Some(null) = buf.iter().position(|b| *b == 0) {
        buf.truncate(null);
    }
    Ok(UTF_8.decode_without_bom_handling(&buf).0.into_owned())
}
