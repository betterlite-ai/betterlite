use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

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
#[serde(into = "PromptLiteLLMParamsJson")]
pub struct PromptLiteLLMParams {
    pub prompt_id: Option<String>,
    pub prompt_integration: String,
    pub api_base: Option<String>,
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

// The database row stores the key the way LiteLLM's model_dump_json does, so this copy exposes the secret and a response copy masks it first
#[derive(Clone, Serialize)]
struct PromptLiteLLMParamsJson {
    prompt_id: Option<String>,
    prompt_integration: String,
    api_base: Option<String>,
    api_key: Option<String>,
    provider_specific_query_params: Option<IndexMap<String, Json>>,
    #[serde(rename = "ignore_prompt_manager_model")]
    ignore_model: bool,
    #[serde(rename = "ignore_prompt_manager_optional_params")]
    ignore_optional_params: bool,
    dotprompt_content: Option<String>,
    #[serde(flatten)]
    extra: IndexMap<String, Json>,
}

impl From<PromptLiteLLMParams> for PromptLiteLLMParamsJson {
    fn from(params: PromptLiteLLMParams) -> Self {
        let api_key = params.api_key.map(|key| key.expose_secret().to_owned());
        Self {
            prompt_id: params.prompt_id,
            prompt_integration: params.prompt_integration,
            api_base: params.api_base,
            api_key,
            provider_specific_query_params: params.provider_specific_query_params,
            ignore_model: params.ignore_model,
            ignore_optional_params: params.ignore_optional_params,
            dotprompt_content: params.dotprompt_content,
            extra: params.extra,
        }
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
