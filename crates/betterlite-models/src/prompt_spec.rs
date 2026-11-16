use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::ser::{SerializeMap as _, Serializer};
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

#[derive(Clone, Debug, Deserialize)]
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

// The database row stores the key the way LiteLLM's model_dump_json does, so the secret is exposed here and a response copy masks it first
impl Serialize for PromptLiteLLMParams {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("prompt_id", &self.prompt_id)?;
        map.serialize_entry("prompt_integration", &self.prompt_integration)?;
        map.serialize_entry("api_base", &self.api_base)?;
        map.serialize_entry(
            "api_key",
            &self
                .api_key
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
        )?;
        map.serialize_entry(
            "provider_specific_query_params",
            &self.provider_specific_query_params,
        )?;
        map.serialize_entry("ignore_prompt_manager_model", &self.ignore_model)?;
        map.serialize_entry(
            "ignore_prompt_manager_optional_params",
            &self.ignore_optional_params,
        )?;
        map.serialize_entry("dotprompt_content", &self.dotprompt_content)?;
        for (name, value) in &self.extra {
            map.serialize_entry(name, value)?;
        }
        map.end()
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
