use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::Headers;
use serde::{Deserialize, Serialize};

use super::Mode;

// litellm.transcription's arguments, as the case files write them; the harness supplies the api_base
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub mode: Mode,
    pub model: Option<String>,
    #[serde(default)]
    pub missing_model: bool,
    pub file: File,
    pub language: Option<String>,
    pub prompt: Option<String>,
    pub response_format: Option<String>,
    pub timestamp_granularities: Option<Vec<String>>,
    pub temperature: Option<i64>,
    pub user: Option<String>,
    pub api_key: Option<String>,
    pub api_version: Option<String>,
    pub timeout_seconds: Option<f64>,
    pub max_retries: Option<i64>,
    pub custom_llm_provider: Option<String>,
    pub base_url: Option<String>,
    pub azure_ad_token: Option<String>,
    pub extra_headers: Option<Headers>,
    pub params: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub filename: String,
    pub content: String,
}
