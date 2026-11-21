#![expect(
    clippy::disallowed_types,
    reason = "a health check deployment carries provider parameters betterlite does not model, kept in their given order and passed to the provider unchanged"
)]
use super::error::HealthProbeError;
use super::logging::HealthProbeLogging;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;
use std::sync::Arc;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Param<T> {
    Absent,
    Null,
    Value(T),
}
impl<T> Default for Param<T> {
    fn default() -> Self {
        Self::Absent
    }
}
impl<T> Param<T> {
    #[must_use]
    pub const fn as_option(&self) -> Option<&T> {
        match self {
            Self::Absent | Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
}
#[derive(Clone, Default)]
pub struct HealthProbeParams {
    pub model: Param<String>,
    pub api_base: Param<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Param<String>,
    pub custom_llm_provider: Param<String>,
    pub max_tokens: Param<Json>,
    pub voice: Param<Json>,
    pub messages: Param<Json>,
    pub fallbacks: Param<Json>,
    pub litellm_metadata: Param<Json>,
    pub cache: Param<Json>,
    pub parameters: IndexMap<String, Value>,
    pub litellm_logging_obj: Option<Arc<dyn HealthProbeLogging>>,
}
impl HealthProbeParams {
    pub fn from_map(parameters: IndexMap<String, Value>) -> Result<Self, HealthProbeError> {
        let mut params = Self {
            parameters,
            ..Self::default()
        };
        params.model = params.take_text("model")?;
        params.api_base = params.take_text("api_base")?;
        params.api_version = params.take_text("api_version")?;
        params.custom_llm_provider = params.take_text("custom_llm_provider")?;
        params.api_key = params.take_secret("api_key")?;
        params.max_tokens = params.take_json("max_tokens");
        params.voice = params.take_json("voice");
        params.messages = params.take_json("messages");
        params.fallbacks = params.take_json("fallbacks");
        params.litellm_metadata = params.take_json("litellm_metadata");
        params.cache = params.take_json("cache");
        Ok(params)
    }
    fn take_text(&mut self, key: &'static str) -> Result<Param<String>, HealthProbeError> {
        match self.parameters.shift_remove(key) {
            None => Ok(Param::Absent),
            Some(Value::Null) => Ok(Param::Null),
            Some(Value::String(text)) => Ok(Param::Value(text)),
            Some(_) => Err(HealthProbeError::ParameterType { key }),
        }
    }
    fn take_secret(&mut self, key: &'static str) -> Result<Option<SecretString>, HealthProbeError> {
        match self.parameters.shift_remove(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(SecretString::from(text))),
            Some(_) => Err(HealthProbeError::ParameterType { key }),
        }
    }
    fn take_json(&mut self, key: &'static str) -> Param<Json> {
        match self.parameters.shift_remove(key) {
            None => Param::Absent,
            Some(value) => Param::Value(json_from_value(value)),
        }
    }
    // the provider call kwargs, without api_key and the logging object: both travel as typed fields
    #[must_use]
    pub fn call_parameters(&self, include_messages: bool) -> IndexMap<String, Json> {
        let mut parameters = IndexMap::new();
        insert_text(&mut parameters, "model", &self.model);
        insert_text(&mut parameters, "api_base", &self.api_base);
        insert_text(&mut parameters, "api_version", &self.api_version);
        insert_text(
            &mut parameters,
            "custom_llm_provider",
            &self.custom_llm_provider,
        );
        insert_json(&mut parameters, "max_tokens", &self.max_tokens);
        insert_json(&mut parameters, "voice", &self.voice);
        if include_messages {
            insert_json(&mut parameters, "messages", &self.messages);
        }
        insert_json(&mut parameters, "fallbacks", &self.fallbacks);
        insert_json(&mut parameters, "litellm_metadata", &self.litellm_metadata);
        insert_json(&mut parameters, "cache", &self.cache);
        for (key, value) in &self.parameters {
            if include_messages || key != "messages" {
                parameters.insert(key.clone(), json_from_value(value.clone()));
            }
        }
        parameters
    }
}
fn insert_text(parameters: &mut IndexMap<String, Json>, key: &'static str, value: &Param<String>) {
    match value {
        Param::Absent => {}
        Param::Null => {
            parameters.insert(key.to_owned(), Json::Null);
        }
        Param::Value(text) => {
            parameters.insert(key.to_owned(), Json::Text(text.clone()));
        }
    }
}
fn insert_json(parameters: &mut IndexMap<String, Json>, key: &'static str, value: &Param<Json>) {
    match value {
        Param::Absent => {}
        Param::Null => {
            parameters.insert(key.to_owned(), Json::Null);
        }
        Param::Value(json) => {
            parameters.insert(key.to_owned(), json.clone());
        }
    }
}
fn json_from_value(value: Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(flag),
        Value::Number(number) => Json::Number(number),
        Value::String(text) => Json::Text(text),
        Value::Array(items) => Json::List(items.into_iter().map(json_from_value).collect()),
        Value::Object(entries) => Json::Object(
            entries
                .into_iter()
                .map(|(key, entry)| (key, json_from_value(entry)))
                .collect(),
        ),
    }
}
