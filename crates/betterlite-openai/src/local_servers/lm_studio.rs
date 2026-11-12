use secrecy::SecretString;
use serde_json::Value;

use super::{ProviderEnvironment, resolve_api_base, resolve_api_key};
use crate::openai_base_config::{CatalogCapabilities, Object, OpenAiConfig};

const PLACEHOLDER_KEY: &str = "fake-api-key";

// LmStudioEmbeddingConfig takes no parameters, keeps no config and returns optional_params unchanged
pub const EMBEDDING_SUPPORTED_PARAMS: [&str; 0] = [];
pub const EMBEDDING_CONFIG_KEYS: [&str; 0] = [];

// the base config is OpenAIGPTConfig, so its family is Other and reasoning_effort stays unsupported
#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub base: OpenAiConfig,
}

#[derive(Debug)]
pub struct ProviderInfo {
    pub api_base: Option<String>,
    pub api_key: SecretString,
}

#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment: &ProviderEnvironment,
) -> ProviderInfo {
    ProviderInfo {
        api_base: resolve_api_base(api_base, environment),
        api_key: resolve_api_key(api_key, environment, PLACEHOLDER_KEY),
    }
}

fn json_format(format: &Object) -> Option<Value> {
    match format.get("type").and_then(Value::as_str) {
        Some("json_schema") => Some(match (format.get("json_schema"), format.get("schema")) {
            (None, Some(schema)) => {
                let inner = Object::from_iter([(String::from("schema"), schema.clone())]);
                Value::Object(Object::from_iter([
                    (
                        String::from("type"),
                        Value::String(String::from("json_schema")),
                    ),
                    (String::from("json_schema"), Value::Object(inner)),
                ]))
            }
            _ => Value::Object(format.clone()),
        }),
        Some("json_object") => Some(Value::Object(format.clone())),
        _ => None,
    }
}

impl ChatConfig {
    pub fn map_openai_params(
        &self,
        model: &str,
        non_default_params: &Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        let mut remaining = non_default_params.clone();
        let rewritten = match non_default_params.get("response_format") {
            Some(Value::Object(format)) => json_format(format),
            _ => None,
        };
        if let Some(format) = rewritten {
            optional_params.insert(String::from("response_format"), format);
            remaining.remove("response_format");
        }
        self.base
            .map_params(model, &remaining, optional_params, catalog);
    }
}
