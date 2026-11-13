use betterlite_config::sections::agent_host_settings::Settings;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::openai_base_config::{Headers, Object};

pub const SERVERLESS_ENDPOINT: &str = "https://inference.do-ai.run";
const AGENT_SUFFIX: &str = "/api/v1/chat/completions";
const SERVERLESS_SUFFIX: &str = "/v1/chat/completions";

pub const SUPPORTED_PARAMS: &[&str] = &[
    "frequency_penalty",
    "max_tokens",
    "max_completion_tokens",
    "presence_penalty",
    "stop",
    "stream",
    "stream_options",
    "temperature",
    "top_p",
    "k",
    "kb_filters",
    "filter_kb_content_by_query_metadata",
    "instruction_override",
    "include_functions_info",
    "include_retrieval_info",
    "include_guardrails_info",
    "provide_citations",
    "retrieval_method",
];

// B668's class snapshot: every constructor field starts absent and a given value is stored under its own name
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GradientSettings {
    pub frequency_penalty: Option<Value>,
    pub max_tokens: Option<Value>,
    pub max_completion_tokens: Option<Value>,
    pub presence_penalty: Option<Value>,
    pub retrieval_method: Option<Value>,
    pub stop: Option<Value>,
    pub stream: Option<Value>,
    pub temperature: Option<Value>,
    pub top_p: Option<Value>,
    #[serde(rename = "k")]
    pub result_count: Option<Value>,
    pub kb_filters: Option<Value>,
    #[serde(rename = "filter_kb_content_by_query_metadata")]
    pub filter_kb_by_query_metadata: Option<Value>,
    pub instruction_override: Option<Value>,
    pub include_functions_info: Option<Value>,
    pub include_retrieval_info: Option<Value>,
    pub include_guardrails_info: Option<Value>,
    pub provide_citations: Option<Value>,
}

impl GradientSettings {
    #[must_use]
    pub fn get_config(&self) -> Object {
        let mut config = Object::new();
        for (name, value) in [
            ("frequency_penalty", &self.frequency_penalty),
            ("max_tokens", &self.max_tokens),
            ("max_completion_tokens", &self.max_completion_tokens),
            ("presence_penalty", &self.presence_penalty),
            ("retrieval_method", &self.retrieval_method),
            ("stop", &self.stop),
            ("stream", &self.stream),
            ("temperature", &self.temperature),
            ("top_p", &self.top_p),
            ("k", &self.result_count),
            ("kb_filters", &self.kb_filters),
            (
                "filter_kb_content_by_query_metadata",
                &self.filter_kb_by_query_metadata,
            ),
            ("instruction_override", &self.instruction_override),
            ("include_functions_info", &self.include_functions_info),
            ("include_retrieval_info", &self.include_retrieval_info),
            ("include_guardrails_info", &self.include_guardrails_info),
            ("provide_citations", &self.provide_citations),
        ] {
            if let Some(stored) = value {
                config.insert(name.to_owned(), stored.clone());
            }
        }
        config
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GradientError {
    #[error("GradientAI API key not found")]
    MissingApiKey,
    #[error("GradientAI does not support the {name} parameter")]
    UnsupportedParam { name: String },
}

impl GradientError {
    // UnsupportedParamsError carries 400; the missing-key check raises a bare ValueError with no status
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsupportedParam { .. } => Some(400),
            Self::MissingApiKey => None,
        }
    }
}

pub struct GradientAiConfig;

impl GradientAiConfig {
    // the environment key is read again here, and only a missing key is rejected, so an empty one is used as sent
    pub fn validate_environment(
        settings: &Settings,
        api_key: Option<&SecretString>,
        headers: Option<Headers>,
    ) -> Result<Headers, GradientError> {
        let key = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(settings.gradient_ai_api_key.as_ref())
            .ok_or(GradientError::MissingApiKey)?;
        let mut validated = headers.unwrap_or_default();
        validated.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
        validated.insert("Content-Type".to_owned(), "application/json".to_owned());
        Ok(validated)
    }

    // LiteLLM reads the agent endpoint a second time, so it also replaces an explicit serverless base
    #[must_use]
    pub fn get_complete_url(settings: &Settings, api_base: Option<&str>) -> String {
        let base = api_base.filter(|value| !value.is_empty() && *value != SERVERLESS_ENDPOINT);
        if let Some(custom_base) = base {
            return format!("{custom_base}{AGENT_SUFFIX}");
        }
        let agent_endpoint = settings
            .gradient_ai_agent_endpoint
            .as_deref()
            .filter(|value| !value.is_empty() && *value != SERVERLESS_ENDPOINT);
        if let Some(agent) = agent_endpoint {
            return format!("{agent}{AGENT_SUFFIX}");
        }
        format!("{SERVERLESS_ENDPOINT}{SERVERLESS_SUFFIX}")
    }

    // Gradient's override ignores the replace_max_completion_tokens flag, so both token-limit names stay as passed
    pub fn map_params(
        non_default: &IndexMap<String, Value>,
        existing: &mut Object,
        drop_params: bool,
    ) -> Result<(), GradientError> {
        for (name, value) in non_default {
            if SUPPORTED_PARAMS.contains(&name.as_str()) {
                existing.insert(name.clone(), value.clone());
                continue;
            }
            if !drop_params {
                return Err(GradientError::UnsupportedParam { name: name.clone() });
            }
        }
        Ok(())
    }
}
