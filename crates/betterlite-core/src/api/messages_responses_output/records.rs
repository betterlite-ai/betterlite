use betterlite_models::projections::reasoning_effort::ThinkingBudgetThresholds;
use betterlite_openai::api::responses_objects::{ResponseApiUsage, ResponseOutputItem};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_with::{As, DefaultOnError, DefaultOnNull, serde_as};

use crate::api::messages_responses_input::records::{MessageRecord, PromptCacheBreakpoint};

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicRequestRecord {
    pub model: String,
    pub messages: Vec<MessageRecord>,
    #[serde(default)]
    pub system: Option<SystemContent>,
    #[serde(default)]
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub tools: Option<Vec<ToolRecord>>,
    #[serde(default)]
    pub tool_choice: Option<ToolChoiceRecord>,
    #[serde(default)]
    pub thinking: Option<ThinkingRecord>,
    #[serde(default)]
    pub output_config: Option<OutputConfigRecord>,
    #[serde(default)]
    pub output_format: Option<OutputFormatRecord>,
    #[serde(default)]
    pub context_management: Option<ContextManagementRecord>,
    #[serde(default)]
    pub metadata: Option<MetadataRecord>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SettingsRecord {
    pub reasoning_auto_summary: bool,
    pub budget_thresholds: ThinkingBudgetThresholds,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SystemContent {
    Text(String),
    Blocks(Vec<SystemBlock>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemBlock {
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolRecord {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub r#type: String,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Option<Object>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub strict: bool,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolChoiceRecord {
    // an absent, null or unknown type falls back to "auto", like tool_choice.get("type")
    #[serde(default, rename = "type")]
    #[serde_as(as = "DefaultOnError")]
    pub kind: ToolChoiceKind,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoiceKind {
    #[default]
    Auto,
    Any,
    Tool,
    None,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThinkingRecord {
    // thinking is declared as a plain dict, so every value keeps the shape the provider sent
    #[serde(default)]
    pub r#type: Option<Json>,
    #[serde(default)]
    pub budget_tokens: Option<Json>,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputConfigRecord {
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub format: Option<OutputFormatRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputFormatRecord {
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub schema: Option<Object>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub strict: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContextManagementRecord {
    // context_management is declared as a plain dict, so edits keeps the shape the client sent
    #[serde(default)]
    pub edits: Option<Json>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetadataRecord {
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseRecord {
    pub id: String,
    #[serde(default)]
    pub model: Option<String>,
    pub output: Vec<ResponseOutputItem>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub usage: Option<ResponseApiUsage>,
}
