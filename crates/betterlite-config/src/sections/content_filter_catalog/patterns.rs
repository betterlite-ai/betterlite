use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use fancy_regex::Regex;
use serde::Deserialize;

use crate::sections::content_filter_catalog::roots::DATA_DIR;

#[derive(Debug, Clone)]
pub struct PatternMetadata {
    pub name: String,
    pub display_name: String,
    pub category: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PatternExtraConfig {
    pub keyword_pattern: Option<String>,
    pub allow_word_numbers: Option<bool>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PatternCatalogError {
    #[error("failed to read patterns.json: {message}")]
    Read { message: String },
    #[error("failed to parse patterns.json: {message}")]
    Parse { message: String },
    #[error("failed to compile pattern '{name}': {message}")]
    Compile { name: String, message: String },
    #[error("Unknown pattern name: '{name}'. Available patterns: {available}")]
    UnknownPattern { name: String, available: String },
}

#[derive(Debug, Clone, Deserialize)]
struct PatternsFile {
    patterns: Vec<PatternEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct PatternEntry {
    name: String,
    display_name: String,
    pattern: String,
    category: String,
    description: String,
    #[serde(rename = "action")]
    _action: Option<String>,
    #[serde(flatten)]
    extra: PatternExtraConfig,
}

fn load_patterns() -> Result<Vec<PatternEntry>, PatternCatalogError> {
    let path = Path::new(DATA_DIR).join("patterns.json");
    let content = fs::read_to_string(&path).map_err(|source| PatternCatalogError::Read {
        message: source.to_string(),
    })?;
    let parsed: PatternsFile =
        serde_json::from_str(&content).map_err(|source| PatternCatalogError::Parse {
            message: source.to_string(),
        })?;
    Ok(parsed.patterns)
}

pub fn get_compiled_pattern(pattern_name: &str) -> Result<Regex, PatternCatalogError> {
    let patterns = load_patterns()?;
    let Some(entry) = patterns.iter().find(|entry| entry.name == pattern_name) else {
        let available = patterns
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(PatternCatalogError::UnknownPattern {
            name: pattern_name.to_owned(),
            available,
        });
    };
    Regex::new(&format!("(?i){}", entry.pattern)).map_err(|source| PatternCatalogError::Compile {
        name: pattern_name.to_owned(),
        message: source.to_string(),
    })
}

pub fn get_all_pattern_names() -> Result<Vec<String>, PatternCatalogError> {
    let patterns = load_patterns()?;
    let mut names = Vec::with_capacity(patterns.len());
    for entry in patterns {
        names.push(entry.name);
    }
    Ok(names)
}

pub fn get_pattern_metadata() -> Result<Vec<PatternMetadata>, PatternCatalogError> {
    let patterns = load_patterns()?;
    Ok(patterns
        .into_iter()
        .map(|entry| PatternMetadata {
            name: entry.name,
            display_name: entry.display_name,
            category: entry.category,
            description: entry.description,
        })
        .collect())
}

pub fn pattern_categories() -> Result<BTreeMap<String, Vec<String>>, PatternCatalogError> {
    let patterns = load_patterns()?;
    let mut categories: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in patterns {
        categories
            .entry(entry.category)
            .or_default()
            .push(entry.name);
    }
    Ok(categories)
}

pub fn pattern_extra_config() -> Result<BTreeMap<String, PatternExtraConfig>, PatternCatalogError> {
    Ok(load_patterns()?
        .into_iter()
        .map(|entry| (entry.name, entry.extra))
        .collect())
}

mod tests;
