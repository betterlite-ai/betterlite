use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ContentFilterAction;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalCategoryConfig {
    pub identifier_words: Vec<String>,
    pub block_words: Vec<String>,
    pub action: ContentFilterAction,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledPatternEntry {
    pub regex: String,
    pub pattern_name: String,
    pub action: ContentFilterAction,
    #[serde(default)]
    pub keyword_regex: Option<String>,
    pub allow_word_numbers: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct _PatternExtraLookup {
    #[serde(default)]
    pub keyword_pattern: Option<String>,
    pub allow_word_numbers: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct _CategoryConfigView {
    pub category: String,
    pub enabled: bool,
    pub action: ContentFilterAction,
    #[serde(default)]
    pub category_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryFileData {
    #[serde(default)]
    pub category_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_action: Option<String>,
    #[serde(default)]
    pub keywords: Vec<BTreeMap<String, String>>,
    #[serde(default)]
    pub exceptions: Vec<String>,
    #[serde(default)]
    pub identifier_words: Vec<String>,
    #[serde(default)]
    pub always_block_keywords: Vec<BTreeMap<String, String>>,
    #[serde(default)]
    pub inherit_from: Option<String>,
    #[serde(default)]
    pub additional_block_words: Vec<String>,
    #[serde(default)]
    pub phrase_patterns: Vec<String>,
}
