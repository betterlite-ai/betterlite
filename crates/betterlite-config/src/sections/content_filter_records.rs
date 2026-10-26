crate::sections::stub_section!();
#[path = "content_filter_records/tests.rs"]
mod config_types;
pub mod overrides;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentFilterAction {
    #[serde(rename = "BLOCK")]
    Block,
    #[serde(rename = "MASK")]
    Mask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentFilterPattern {
    pub pattern_type: String,
    #[serde(default)]
    pub pattern_name: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub action: ContentFilterAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedWord {
    pub keyword: String,
    pub action: ContentFilterAction,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorIntentResult {
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub entities: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub signals: Vec<String>,
    #[serde(default)]
    pub action_hint: Option<String>,
    #[serde(default)]
    pub evidence: Vec<overrides::CompetitorIntentEvidenceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternDetection {
    pub r#type: String,
    pub pattern_name: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedWordDetection {
    pub r#type: String,
    pub keyword: String,
    pub action: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryKeywordDetection {
    pub r#type: String,
    pub category: String,
    pub keyword: String,
    pub severity: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorIntentDetection {
    pub r#type: String,
    pub intent: String,
    pub confidence: f64,
    pub action_hint: String,
    pub entities: BTreeMap<String, Vec<String>>,
    pub signals: Vec<String>,
    pub evidence: Vec<overrides::CompetitorIntentEvidenceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentFilterDetection {
    #[serde(rename = "pattern")]
    Pattern(PatternDetection),
    #[serde(rename = "blocked_word")]
    BlockedWord(BlockedWordDetection),
    #[serde(rename = "category_keyword")]
    CategoryKeyword(CategoryKeywordDetection),
    #[serde(rename = "competitor_intent")]
    CompetitorIntent(CompetitorIntentDetection),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentFilterCategoryConfig {
    pub category: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub action: ContentFilterAction,
    #[serde(default = "default_severity_threshold")]
    pub severity_threshold: String,
    #[serde(default)]
    pub category_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentFilterGuardrailConfig {
    #[serde(default)]
    pub patterns: Option<Vec<ContentFilterPattern>>,
    #[serde(default)]
    pub blocked_words: Option<Vec<BlockedWord>>,
    #[serde(default)]
    pub blocked_words_file: Option<String>,
    #[serde(default)]
    pub categories: Option<Vec<ContentFilterCategoryConfig>>,
    #[serde(default = "default_severity_threshold")]
    pub severity_threshold: String,
    #[serde(default = "default_pattern_format")]
    pub pattern_redaction_format: String,
    #[serde(default = "default_keyword_tag")]
    pub keyword_redaction_tag: String,
    #[serde(default)]
    pub competitor_intent_config: Option<overrides::CompetitorIntentConfig>,
}

impl Default for ContentFilterGuardrailConfig {
    fn default() -> Self {
        Self {
            patterns: None,
            blocked_words: None,
            blocked_words_file: None,
            categories: None,
            severity_threshold: default_severity_threshold(),
            pattern_redaction_format: String::from("[{pattern_name}_REDACTED]"),
            keyword_redaction_tag: String::from("[KEYWORD_REDACTED]"),
            competitor_intent_config: None,
        }
    }
}

impl ContentFilterGuardrailConfig {
    pub const UI_FRIENDLY_NAME: &'static str = "LiteLLM Content Filter";

    #[must_use]
    pub const fn ui_friendly_name() -> &'static str {
        match Self::UI_FRIENDLY_NAME.as_bytes().first() {
            Some(_) => Self::UI_FRIENDLY_NAME,
            None => "",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlockDetection {
    pub r#type: String,
    pub language: String,
    pub confidence: f64,
    pub action_taken: CodeBlockActionTaken,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeBlockActionTaken {
    Block,
    Allow,
    LogOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockCodeExecGuardrailConfig {
    #[serde(default)]
    pub blocked_languages: Option<Vec<String>>,
    #[serde(default = "default_block_action")]
    pub action: String,
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f64,
    #[serde(default = "default_enabled")]
    pub detect_execution_intent: bool,
}

impl BlockCodeExecGuardrailConfig {
    pub const UI_FRIENDLY_NAME: &'static str = "Block Code Execution";

    #[must_use]
    pub const fn ui_friendly_name() -> &'static str {
        match Self::UI_FRIENDLY_NAME.as_bytes().first() {
            Some(_) => Self::UI_FRIENDLY_NAME,
            None => "",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HideSecretsGuardrailConfig {
    #[serde(default)]
    pub detect_secrets_config: Option<overrides::DetectSecretsConfig>,
}

impl HideSecretsGuardrailConfig {
    pub const UI_FRIENDLY_NAME: &'static str = "Hide Secrets";

    #[must_use]
    pub const fn ui_friendly_name() -> &'static str {
        match Self::UI_FRIENDLY_NAME.as_bytes().first() {
            Some(_) => Self::UI_FRIENDLY_NAME,
            None => "",
        }
    }
}

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

const fn default_enabled() -> bool {
    let enabled = true;
    if enabled { enabled } else { true }
}

fn default_severity_threshold() -> String {
    let threshold = String::from("medium");
    if threshold.is_empty() {
        String::from("medium")
    } else {
        threshold
    }
}

fn default_pattern_format() -> String {
    let format = String::from("[{pattern_name}_REDACTED]");
    if format.is_empty() {
        String::from("[{pattern_name}_REDACTED]")
    } else {
        format
    }
}

fn default_keyword_tag() -> String {
    let tag = String::from("[KEYWORD_REDACTED]");
    if tag.is_empty() {
        String::from("[KEYWORD_REDACTED]")
    } else {
        tag
    }
}

fn default_block_action() -> String {
    let action = String::from("block");
    if action.is_empty() {
        String::from("block")
    } else {
        action
    }
}

const fn default_confidence_threshold() -> f64 {
    let threshold = 0.5_f64;
    if threshold.is_sign_negative() {
        0.0
    } else {
        threshold
    }
}
