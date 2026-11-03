use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

use crate::sections::content_filter_records::category_types::CategoryFileData;

use super::model::{CategoryConfig, CategoryConfigError};

#[derive(Debug, thiserror::Error)]
pub enum CategoryFileError {
    #[error("failed to read category file '{path}': {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse category YAML '{path}': {source}")]
    Yaml {
        path: String,
        #[source]
        source: Box<serde_saphyr::Error>,
    },
    #[error("failed to parse category JSON '{path}': {source}")]
    Json {
        path: String,
        #[source]
        source: Box<serde_json::Error>,
    },
    #[error("invalid category file '{path}': {source}")]
    InvalidConfig {
        path: String,
        #[source]
        source: CategoryConfigError,
    },
}

pub fn read_category_yaml(path: &Path) -> Result<CategoryFileData, CategoryFileError> {
    let text = read_text(path)?;
    serde_saphyr::from_str(&text).map_err(|source| CategoryFileError::Yaml {
        path: path.display().to_string(),
        source: Box::new(source),
    })
}

pub fn load_category_file(path: &Path) -> Result<CategoryConfig, CategoryFileError> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return load_json(path);
    }
    let data = read_category_yaml(path)?;
    CategoryConfig::from_data(data).map_err(|source| CategoryFileError::InvalidConfig {
        path: path.display().to_string(),
        source,
    })
}

fn load_json(path: &Path) -> Result<CategoryConfig, CategoryFileError> {
    let input = parse_json_entries(path)?;
    let entries = match input {
        JsonEntries::Many(entries) => entries,
        JsonEntries::One(entry) => vec![entry],
    };
    CategoryConfig::from_data(CategoryFileData {
        category_name: Some(category_name(path)),
        description: Some("Detects harmful, toxic, or abusive language and content".to_owned()),
        default_action: Some("BLOCK".to_owned()),
        keywords: json_keywords(entries),
        exceptions: Vec::new(),
        identifier_words: Vec::new(),
        always_block_keywords: Vec::new(),
        inherit_from: None,
        additional_block_words: Vec::new(),
        phrase_patterns: Vec::new(),
    })
    .map_err(|source| CategoryFileError::InvalidConfig {
        path: path.display().to_string(),
        source,
    })
}

fn parse_json_entries(path: &Path) -> Result<JsonEntries, CategoryFileError> {
    let text = read_text(path)?;
    serde_json::from_str(&text).map_err(|source| CategoryFileError::Json {
        path: path.display().to_string(),
        source: Box::new(source),
    })
}

fn read_text(path: &Path) -> Result<String, CategoryFileError> {
    fs::read_to_string(path).map_err(|source| CategoryFileError::Read {
        path: path.display().to_string(),
        source,
    })
}

fn json_keywords(entries: Vec<JsonEntry>) -> Vec<BTreeMap<String, String>> {
    let mut seen = BTreeSet::new();
    let mut keywords = Vec::new();
    for entry in entries {
        for phrase in entry
            .match_phrases
            .unwrap_or_default()
            .split('|')
            .map(str::trim)
            .map(str::to_lowercase)
            .filter(|phrase| !phrase.is_empty())
        {
            if seen.insert(phrase.clone()) {
                keywords.push(BTreeMap::from([
                    ("keyword".to_owned(), phrase),
                    (
                        "severity".to_owned(),
                        severity_name(entry.severity.as_ref()).to_owned(),
                    ),
                ]));
            }
        }
    }
    keywords
}

fn category_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unknown")
        .to_owned()
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum JsonEntries {
    Many(Vec<JsonEntry>),
    One(JsonEntry),
}

#[derive(serde::Deserialize)]
struct JsonEntry {
    #[serde(rename = "match")]
    match_phrases: Option<String>,
    severity: Option<JsonSeverity>,
    _id: Option<String>,
    _tags: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum JsonSeverity {
    Number(serde_json::Number),
    Text(String),
}

fn severity_name(severity: Option<&JsonSeverity>) -> &'static str {
    match severity {
        Some(JsonSeverity::Number(number)) if number.as_i64() == Some(1) => "low",
        Some(JsonSeverity::Number(number)) if matches!(number.as_i64(), Some(3 | 4)) => "high",
        Some(JsonSeverity::Text(value)) => {
            tracing::debug!(
                severity = value,
                "non-integer category severity defaults to medium"
            );
            "medium"
        }
        None | Some(JsonSeverity::Number(_)) => "medium",
    }
}
