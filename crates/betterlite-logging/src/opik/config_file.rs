mod interpolate;
mod scan;

use std::collections::BTreeMap;

use indexmap::IndexMap;
use thiserror::Error;

use interpolate::interpolate_section;
use scan::scan;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigFileError {
    #[error("line {line} appears before any section header")]
    BeforeHeader { line: usize },
    #[error("line {line} in section {section} has no = or :")]
    MissingDelimiter { section: String, line: usize },
    #[error("line {line} in section {section} has an empty key")]
    EmptyKey { section: String, line: usize },
    #[error("section {section} is repeated at line {line}")]
    DuplicateSection { section: String, line: usize },
    #[error("option {option} in section {section} is repeated at line {line}")]
    DuplicateOption {
        section: String,
        option: String,
        line: usize,
    },
    #[error("interpolation in section {section} option {option} nests past depth {depth}")]
    InterpolationDepth {
        section: String,
        option: String,
        depth: u32,
    },
    #[error("interpolation in section {section} option {option} names an unknown option")]
    InterpolationMissingName { section: String, option: String },
    #[error("interpolation in section {section} option {option} has a malformed % reference")]
    InterpolationSyntax { section: String, option: String },
}

pub fn read_config_section(contents: &str) -> Result<BTreeMap<String, String>, ConfigFileError> {
    let scan = scan(contents)?;
    let defaults = join_section(scan.defaults);
    let mut opik_result: Option<IndexMap<String, String>> = None;
    for (name, raw_section) in scan.sections {
        let joined = join_section(raw_section);
        let merged = merge_with_defaults(&joined, &defaults);
        let interpolated = interpolate_section(&name, &merged)?;
        if name == "opik" {
            opik_result = Some(interpolated);
        }
    }
    Ok(opik_result.unwrap_or_default().into_iter().collect())
}

fn join_section(raw: IndexMap<String, Vec<String>>) -> IndexMap<String, String> {
    let mut joined = IndexMap::new();
    for (key, lines) in raw {
        joined.insert(key, lines.join("\n").trim_end().to_owned());
    }
    joined
}

fn merge_with_defaults(
    section: &IndexMap<String, String>,
    defaults: &IndexMap<String, String>,
) -> IndexMap<String, String> {
    let mut merged = defaults.clone();
    for (key, value) in section {
        merged.insert(key.clone(), value.clone());
    }
    merged
}
