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
impl<T> Param<T> {
    #[must_use]
    pub const fn as_option(&self) -> Option<&T> {
        match self {
            Self::Absent | Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
}
#[derive(Clone)]
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
impl Default for HealthProbeParams {
    fn default() -> Self {
        Self {
            model: Param::Absent,
            api_base: Param::Absent,
            api_key: None,
            api_version: Param::Absent,
            custom_llm_provider: Param::Absent,
            max_tokens: Param::Absent,
            voice: Param::Absent,
            messages: Param::Absent,
            fallbacks: Param::Absent,
            litellm_metadata: Param::Absent,
            cache: Param::Absent,
            parameters: IndexMap::new(),
            litellm_logging_obj: None,
        }
    }
}
impl HealthProbeParams {
    pub fn from_map(mut parameters: IndexMap<String, Value>) -> Result<Self, HealthProbeError> {
        let model = take_text(&mut parameters, "model")?;
        let api_base = take_text(&mut parameters, "api_base")?;
        let api_version = take_text(&mut parameters, "api_version")?;
        let custom_llm_provider = take_text(&mut parameters, "custom_llm_provider")?;
        let api_key = take_text(&mut parameters, "api_key")?
            .as_option()
            .cloned()
            .map(SecretString::from);
        let max_tokens = take_json(&mut parameters, "max_tokens")?;
        let voice = take_json(&mut parameters, "voice")?;
        let messages = take_json(&mut parameters, "messages")?;
        let fallbacks = take_json(&mut parameters, "fallbacks")?;
        let litellm_metadata = take_json(&mut parameters, "litellm_metadata")?;
        let cache = take_json(&mut parameters, "cache")?;
        let params = Self {
            model,
            api_base,
            api_key,
            api_version,
            custom_llm_provider,
            max_tokens,
            voice,
            messages,
            fallbacks,
            litellm_metadata,
            cache,
            parameters,
            litellm_logging_obj: None,
        };
        Ok(params)
    }
    // the provider call kwargs, without api_key and the logging object: both travel as typed fields
    pub fn call_parameters(
        &self,
        include_messages: bool,
    ) -> Result<IndexMap<String, Json>, HealthProbeError> {
        let mut parameters = IndexMap::new();
        insert_parameter(&mut parameters, "model", &self.model);
        insert_parameter(&mut parameters, "api_base", &self.api_base);
        insert_parameter(&mut parameters, "api_version", &self.api_version);
        insert_parameter(
            &mut parameters,
            "custom_llm_provider",
            &self.custom_llm_provider,
        );
        insert_parameter(&mut parameters, "max_tokens", &self.max_tokens);
        insert_parameter(&mut parameters, "voice", &self.voice);
        if include_messages {
            insert_parameter(&mut parameters, "messages", &self.messages);
        }
        insert_parameter(&mut parameters, "fallbacks", &self.fallbacks);
        insert_parameter(&mut parameters, "litellm_metadata", &self.litellm_metadata);
        insert_parameter(&mut parameters, "cache", &self.cache);
        for (key, value) in &self.parameters {
            if include_messages || key != "messages" {
                let typed = serde_json::from_value(value.clone()).map_err(|source| {
                    HealthProbeError::ParameterValue {
                        key: key.clone(),
                        source,
                    }
                })?;
                parameters.insert(key.clone(), typed);
            }
        }
        Ok(parameters)
    }
}
fn take_text(
    parameters: &mut IndexMap<String, Value>,
    key: &'static str,
) -> Result<Param<String>, HealthProbeError> {
    match parameters.shift_remove(key) {
        None => Ok(Param::Absent),
        Some(Value::Null) => Ok(Param::Null),
        Some(Value::String(text)) => Ok(Param::Value(text)),
        Some(_) => Err(HealthProbeError::ParameterType { key }),
    }
}
fn take_json(
    parameters: &mut IndexMap<String, Value>,
    key: &'static str,
) -> Result<Param<Json>, HealthProbeError> {
    match parameters.shift_remove(key) {
        None => Ok(Param::Absent),
        Some(value) => {
            let typed = serde_json::from_value(value).map_err(|source| {
                HealthProbeError::ParameterValue {
                    key: key.to_owned(),
                    source,
                }
            })?;
            Ok(Param::Value(typed))
        }
    }
}
trait ParameterJson {
    fn parameter_json(&self) -> Json;
}
impl ParameterJson for String {
    fn parameter_json(&self) -> Json {
        Json::Text(self.clone())
    }
}
impl ParameterJson for Json {
    fn parameter_json(&self) -> Json {
        self.clone()
    }
}
fn insert_parameter<T>(parameters: &mut IndexMap<String, Json>, key: &'static str, value: &Param<T>)
where
    T: ParameterJson,
{
    match value {
        Param::Absent => {}
        Param::Null => {
            parameters.insert(key.to_owned(), Json::Null);
        }
        Param::Value(inner) => {
            parameters.insert(key.to_owned(), inner.parameter_json());
        }
    }
}
