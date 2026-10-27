crate::sections::stub_section!();
pub mod category_types;
#[path = "content_filter_records/tests.rs"]
mod config_types;
pub mod overrides;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentFilterAction {
    #[serde(rename = "BLOCK", alias = "block")]
    Block,
    #[serde(rename = "MASK", alias = "mask")]
    Mask,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentFilterSeverity {
    High,
    #[default]
    Medium,
    Low,
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
    Pattern(PatternDetection),
    BlockedWord(BlockedWordDetection),
    CategoryKeyword(CategoryKeywordDetection),
    CompetitorIntent(CompetitorIntentDetection),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentFilterCategoryConfig {
    pub category: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub action: ContentFilterAction,
    #[serde(default)]
    pub severity_threshold: ContentFilterSeverity,
    #[serde(default)]
    pub category_file: Option<String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_pattern_redaction_format")]
    pub pattern_redaction_format: String,
    #[serde(default = "default_keyword_redaction_tag")]
    pub keyword_redaction_tag: String,
    #[serde(default)]
    pub competitor_intent_config: Option<overrides::CompetitorIntentConfig>,
}

fn default_severity_threshold() -> String {
    String::from("medium")
}

fn default_pattern_redaction_format() -> String {
    String::from("[{pattern_name}_REDACTED]")
}

fn default_keyword_redaction_tag() -> String {
    String::from("[KEYWORD_REDACTED]")
}

impl ContentFilterGuardrailConfig {
    pub const UI_FRIENDLY_NAME: &'static str = "LiteLLM Content Filter";
}

impl GuardrailConfigModelIdentity for ContentFilterGuardrailConfig {
    fn ui_friendly_name() -> &'static str {
        "LiteLLM Content Filter"
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockCodeAction {
    #[default]
    Block,
    Mask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockCodeExecGuardrailConfig {
    #[serde(default)]
    pub blocked_languages: Option<Vec<String>>,
    #[serde(default)]
    pub action: BlockCodeAction,
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f64,
    #[serde(default = "default_true")]
    pub detect_execution_intent: bool,
}

const fn default_confidence_threshold() -> f64 {
    0.5
}

impl BlockCodeExecGuardrailConfig {
    pub const UI_FRIENDLY_NAME: &'static str = "Block Code Execution";
}

impl GuardrailConfigModelIdentity for BlockCodeExecGuardrailConfig {
    fn ui_friendly_name() -> &'static str {
        "Block Code Execution"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HideSecretsGuardrailConfig {
    #[serde(default)]
    pub detect_secrets_config: Option<overrides::DetectSecretsConfig>,
}

impl HideSecretsGuardrailConfig {
    pub const UI_FRIENDLY_NAME: &'static str = "Hide Secrets";
}

impl GuardrailConfigModelIdentity for HideSecretsGuardrailConfig {
    fn ui_friendly_name() -> &'static str {
        "Hide Secrets"
    }
}
