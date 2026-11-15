use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::registry::prompt_registry_helpers::DEFAULT_PROMPT_ENVIRONMENT;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PromptRowJson {
    Text(String),
    Object(IndexMap<String, Json>),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PromptRow {
    pub id: String,
    pub prompt_id: String,
    pub version: i64,
    pub environment: String,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub litellm_params: PromptRowJson,
    pub prompt_info: Option<PromptRowJson>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PromptRowData {
    pub prompt_id: String,
    #[serde(default = "default_version")]
    pub version: i64,
    #[serde(default = "default_environment")]
    pub environment: String,
    pub created_by: Option<String>,
    pub litellm_params: Option<PromptRowJson>,
    pub prompt_info: Option<PromptRowJson>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

const fn default_version() -> i64 {
    1
}

fn default_environment() -> String {
    DEFAULT_PROMPT_ENVIRONMENT.to_owned()
}
