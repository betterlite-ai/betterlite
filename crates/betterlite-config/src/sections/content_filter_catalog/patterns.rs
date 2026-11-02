use std::collections::BTreeMap;

use fancy_regex::Regex;
use serde::Deserialize;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PatternMetadata {
    pub name: String,
    pub display_name: String,
    pub category: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub struct PatternExtraConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_word_numbers: Option<bool>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PatternCatalogError {
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct PatternCatalog {
    pub names: Vec<String>,
    pub metadata: Vec<PatternMetadata>,
    pub patterns: BTreeMap<String, String>,
    pub categories: BTreeMap<String, Vec<String>>,
    pub display_names: BTreeMap<String, String>,
    pub descriptions: BTreeMap<String, String>,
    pub extra_config: BTreeMap<String, PatternExtraConfig>,
}

const PATTERNS_JSON: &str = include_str!("../../../assets/content_filter/patterns.json");

fn load_patterns() -> Result<Vec<PatternEntry>, PatternCatalogError> {
    let parsed: PatternsFile =
        serde_json::from_str(PATTERNS_JSON).map_err(|source| PatternCatalogError::Parse {
            message: source.to_string(),
        })?;
    Ok(parsed.patterns)
}

pub fn get_compiled_pattern(pattern_name: &str) -> Result<Regex, PatternCatalogError> {
    let catalog = get_pattern_catalog()?;
    let Some(pattern) = catalog.patterns.get(pattern_name) else {
        return Err(PatternCatalogError::UnknownPattern {
            name: pattern_name.to_owned(),
            available: catalog.names.join(", "),
        });
    };
    Regex::new(&format!("(?i){pattern}")).map_err(|source| PatternCatalogError::Compile {
        name: pattern_name.to_owned(),
        message: source.to_string(),
    })
}

pub fn get_all_pattern_names() -> Result<Vec<String>, PatternCatalogError> {
    match get_pattern_catalog() {
        Ok(catalog) => Ok(catalog.names),
        Err(error) => Err(error),
    }
}

pub fn get_pattern_metadata() -> Result<Vec<PatternMetadata>, PatternCatalogError> {
    match get_pattern_catalog() {
        Ok(catalog) => Ok(catalog.metadata),
        Err(error) => Err(error),
    }
}

pub fn get_pattern_catalog() -> Result<PatternCatalog, PatternCatalogError> {
    match load_patterns() {
        Ok(entries) => Ok(build_pattern_catalog(entries)),
        Err(error) => Err(error),
    }
}

fn build_pattern_catalog(entries: Vec<PatternEntry>) -> PatternCatalog {
    let mut names = Vec::with_capacity(entries.len());
    let mut metadata = Vec::with_capacity(entries.len());
    let mut patterns = BTreeMap::new();
    let mut categories: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut display_names = BTreeMap::new();
    let mut descriptions = BTreeMap::new();
    let mut extra_config = BTreeMap::new();
    for entry in entries {
        names.push(entry.name.clone());
        metadata.push(PatternMetadata {
            name: entry.name.clone(),
            display_name: entry.display_name.clone(),
            category: entry.category.clone(),
            description: entry.description.clone(),
        });
        patterns.insert(entry.name.clone(), entry.pattern);
        categories
            .entry(entry.category)
            .or_default()
            .push(entry.name.clone());
        display_names.insert(entry.name.clone(), entry.display_name);
        descriptions.insert(entry.name.clone(), entry.description);
        extra_config.insert(entry.name, entry.extra);
    }
    PatternCatalog {
        names,
        metadata,
        patterns,
        categories,
        display_names,
        descriptions,
        extra_config,
    }
}

pub fn pattern_categories() -> Result<BTreeMap<String, Vec<String>>, PatternCatalogError> {
    match get_pattern_catalog() {
        Ok(catalog) => Ok(catalog.categories),
        Err(error) => Err(error),
    }
}

pub fn pattern_extra_config() -> Result<BTreeMap<String, PatternExtraConfig>, PatternCatalogError> {
    match get_pattern_catalog() {
        Ok(catalog) => Ok(catalog.extra_config),
        Err(error) => Err(error),
    }
}

mod tests;
