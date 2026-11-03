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

#[derive(Debug, thiserror::Error)]
pub enum PatternCatalogError {
    #[error("failed to parse patterns.json: {source}")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to compile pattern '{name}': {source}")]
    Compile {
        name: String,
        #[source]
        source: fancy_regex::Error,
    },
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
const PYTHON_WORD_BOUNDARY: &str =
    r"(?:(?<=[\p{L}\p{N}_])(?![\p{L}\p{N}_])|(?<![\p{L}\p{N}_])(?=[\p{L}\p{N}_]))";

fn load_patterns() -> Result<Vec<PatternEntry>, PatternCatalogError> {
    let parsed: PatternsFile = serde_json::from_str(PATTERNS_JSON)
        .map_err(|source| PatternCatalogError::Parse { source })?;
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
    Regex::new(&format!("(?i){}", compatible_regex_pattern(pattern))).map_err(|source| {
        PatternCatalogError::Compile {
            name: pattern_name.to_owned(),
            source,
        }
    })
}

fn compatible_regex_pattern(pattern: &str) -> String {
    let expanded_ranges = pattern
        .replace("A-Z", "A-Z\u{130}\u{131}\u{17f}\u{212a}")
        .replace("a-z", "a-z\u{130}\u{131}\u{17f}\u{212a}");
    let mut compatible = String::with_capacity(expanded_ranges.len());
    let mut characters = expanded_ranges.chars();
    let mut in_character_class = false;
    while let Some(character) = characters.next() {
        match character {
            '\\' => match characters.next() {
                Some('b') if !in_character_class => compatible.push_str(PYTHON_WORD_BOUNDARY),
                Some(escaped) => {
                    compatible.push('\\');
                    compatible.push(escaped);
                }
                None => compatible.push('\\'),
            },
            '[' if !in_character_class => {
                in_character_class = true;
                compatible.push(character);
            }
            ']' if in_character_class => {
                in_character_class = false;
                compatible.push(character);
            }
            'I' | 'i' if in_character_class => compatible.push_str("Ii\u{130}\u{131}"),
            'S' | 's' if in_character_class => compatible.push_str("Ss\u{17f}"),
            'K' | 'k' if in_character_class => compatible.push_str("Kk\u{212a}"),
            'I' | 'i' => compatible.push_str("[Ii\u{130}\u{131}]"),
            'S' | 's' => compatible.push_str("[Ss\u{17f}]"),
            'K' | 'k' => compatible.push_str("[Kk\u{212a}]"),
            other => compatible.push(other),
        }
    }
    compatible
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
