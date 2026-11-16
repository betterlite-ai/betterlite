use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize, Serializer};

use crate::decisions::Json;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SupportedPromptIntegrations {
    #[serde(rename = "dotprompt")]
    DotPrompt,
    Langfuse,
    Custom,
    Bitbucket,
    Gitlab,
    GenericPromptManagement,
    #[serde(rename = "arize_phoenix")]
    ArizePhoenix,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PromptInfo {
    #[serde(default = "default_prompt_type")]
    pub prompt_type: String,
    #[serde(default = "default_environment")]
    pub environment: String,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

impl Default for PromptInfo {
    fn default() -> Self {
        Self {
            prompt_type: default_prompt_type(),
            environment: default_environment(),
            extra: IndexMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptLiteLLMParams {
    pub prompt_id: Option<String>,
    pub prompt_integration: String,
    pub api_base: Option<String>,
    #[serde(serialize_with = "serialize_api_key")]
    pub api_key: Option<SecretString>,
    pub provider_specific_query_params: Option<IndexMap<String, Json>>,
    #[serde(default)]
    #[serde(rename = "ignore_prompt_manager_model")]
    pub ignore_model: bool,
    #[serde(default)]
    #[serde(rename = "ignore_prompt_manager_optional_params")]
    pub ignore_optional_params: bool,
    pub dotprompt_content: Option<String>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

// The database row stores the key the way LiteLLM's model_dump_json does; serde hands the field as &Option, and a pub fn keeps clippy's ref_option quiet
pub fn serialize_api_key<S>(
    api_key: &Option<SecretString>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match api_key {
        Some(key) => serializer.serialize_some(key.expose_secret()),
        None => serializer.serialize_none(),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptSpec {
    pub prompt_id: String,
    pub litellm_params: PromptLiteLLMParams,
    #[serde(default)]
    pub prompt_info: PromptInfo,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub version: Option<i64>,
    #[serde(default = "default_environment")]
    pub environment: String,
    pub created_by: Option<String>,
}

fn default_environment() -> String {
    let environment = "development";
    environment.to_owned()
}

fn default_prompt_type() -> String {
    let prompt_type = "config";
    prompt_type.to_owned()
}

mod tests;
