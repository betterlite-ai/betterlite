crate::sections::stub_section!();
pub mod category_types;
#[path = "content_filter_records/tests.rs"]
mod config_types;
pub mod overrides;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentFilterAction {
    #[serde(rename = "BLOCK", alias = "block")]
    #[default]
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
#[serde(default)]
pub struct ContentFilterCategoryConfig {
    pub category: String,
    pub enabled: bool,
    pub action: ContentFilterAction,
    pub severity_threshold: ContentFilterSeverity,
    pub category_file: Option<String>,
}

impl Default for ContentFilterCategoryConfig {
    fn default() -> Self {
        let category = String::default();
        let category_file = Option::<String>::default();
        Self {
            category_file,
            severity_threshold: ContentFilterSeverity::default(),
            action: ContentFilterAction::default(),
            enabled: true,
            category,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentFilterGuardrailConfig {
    pub patterns: Option<Vec<ContentFilterPattern>>,
    pub blocked_words: Option<Vec<BlockedWord>>,
    pub blocked_words_file: Option<String>,
    pub categories: Option<Vec<ContentFilterCategoryConfig>>,
    pub severity_threshold: String,
    pub pattern_redaction_format: String,
    pub keyword_redaction_tag: String,
    pub competitor_intent_config: Option<overrides::CompetitorIntentConfig>,
}

impl Default for ContentFilterGuardrailConfig {
    fn default() -> Self {
        let patterns = Option::<Vec<ContentFilterPattern>>::default();
        let blocked_words = Option::<Vec<BlockedWord>>::default();
        let blocked_words_file = Option::<String>::default();
        let categories = Option::<Vec<ContentFilterCategoryConfig>>::default();
        let competitor_intent_config = Option::<overrides::CompetitorIntentConfig>::default();
        Self {
            patterns,
            blocked_words,
            blocked_words_file,
            categories,
            severity_threshold: String::from("medium"),
            pattern_redaction_format: String::from("[{pattern_name}_REDACTED]"),
            keyword_redaction_tag: String::from("[KEYWORD_REDACTED]"),
            competitor_intent_config,
        }
    }
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
#[serde(default)]
pub struct BlockCodeExecGuardrailConfig {
    pub blocked_languages: Option<Vec<String>>,
    pub action: BlockCodeAction,
    pub confidence_threshold: f64,
    pub detect_execution_intent: bool,
}

impl Default for BlockCodeExecGuardrailConfig {
    fn default() -> Self {
        Self {
            blocked_languages: None,
            action: BlockCodeAction::Block,
            confidence_threshold: Self::CONFIDENCE_THRESHOLD,
            detect_execution_intent: Self::DETECT_EXECUTION_INTENT,
        }
    }
}

impl BlockCodeExecGuardrailConfig {
    const CONFIDENCE_THRESHOLD: f64 = 0.5;
    const DETECT_EXECUTION_INTENT: bool = true;
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
