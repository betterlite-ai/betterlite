use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Number;

use super::error::OllamaGenerateError;
use crate::ollama_chat::error::OllamaError;

pub const DEFAULT_API_BASE: &str = "http://localhost:11434";
pub const GENERATE_PATH: &str = "/api/generate";
pub const SUPPORTED_OPENAI_PARAMS: [&str; 10] = [
    "max_tokens",
    "stream",
    "top_p",
    "temperature",
    "seed",
    "frequency_penalty",
    "stop",
    "response_format",
    "max_completion_tokens",
    "reasoning_effort",
];

const GPT_OSS_PREFIX: &str = "gpt-oss";
const REASONING_EFFORTS: [&str; 3] = ["low", "medium", "high"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProviderField {
    pub field_name: &'static str,
    pub field_type: &'static str,
    pub field_description: &'static str,
    pub field_value: &'static str,
}

pub const REQUIRED_PARAMS: [ProviderField; 1] = [ProviderField {
    field_name: "base_url",
    field_type: "string",
    field_description: "Your Ollama API Base",
    field_value: "http://10.10.11.249:11434",
}];

// the deployment-level Ollama options LiteLLM keeps as class attributes and get_config reports
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OllamaGenerateConfig {
    pub mirostat: Option<Number>,
    pub mirostat_eta: Option<Number>,
    pub mirostat_tau: Option<Number>,
    pub num_ctx: Option<Number>,
    pub num_gqa: Option<Number>,
    pub num_gpu: Option<Number>,
    pub num_thread: Option<Number>,
    pub repeat_last_n: Option<Number>,
    pub repeat_penalty: Option<Number>,
    pub temperature: Option<Number>,
    pub seed: Option<Number>,
    pub stop: Option<Vec<String>>,
    pub tfs_z: Option<Number>,
    pub num_predict: Option<Number>,
    pub top_k: Option<Number>,
    pub top_p: Option<Number>,
    pub system: Option<String>,
    pub template: Option<String>,
}

impl OllamaGenerateConfig {
    // LiteLLM's get_config reports the attributes that are set, in definition order
    #[must_use]
    pub fn get_config(&self) -> IndexMap<String, Json> {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        let parsed = serde_json::from_slice::<Json>(&bytes).unwrap_or(Json::Null);
        let Json::Object(fields) = parsed else {
            return IndexMap::new();
        };
        fields
            .into_iter()
            .filter_map(|(name, value)| (!matches!(value, Json::Null)).then_some((name, value)))
            .collect()
    }
}

pub fn map_openai_params(
    non_default: &IndexMap<String, Json>,
    optional: &mut IndexMap<String, Json>,
    model: &str,
) -> Result<(), OllamaGenerateError> {
    for (param, value) in non_default {
        match param.as_str() {
            "max_tokens" | "max_completion_tokens" => {
                optional.insert(String::from("num_predict"), value.clone());
            }
            "stream" | "temperature" | "seed" | "top_p" | "frequency_penalty" | "stop" => {
                optional.insert(param.clone(), value.clone());
            }
            "response_format" => {
                if let Some(format) = response_format(value)? {
                    optional.insert(String::from("format"), format);
                }
            }
            "reasoning_effort" => {
                if let Some(think) = reasoning_effort(model, value)? {
                    optional.insert(String::from("think"), think);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// the generate mapper indexes response_format["type"] directly, so a missing key is an error
fn response_format(value: &Json) -> Result<Option<Json>, OllamaError> {
    let Json::Object(fields) = value else {
        return Ok(None);
    };
    match fields.get("type") {
        None => Err(OllamaError::Param {
            field: "response_format.type",
        }),
        Some(Json::Text(kind)) if kind == "json_object" => {
            Ok(Some(Json::Text(String::from("json"))))
        }
        Some(Json::Text(kind)) if kind == "json_schema" => json_schema_format(fields),
        Some(Json::Text(_) | Json::Null) => Ok(None),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(OllamaError::Param {
                field: "response_format.type",
            })
        }
    }
}

fn json_schema_format(fields: &Object) -> Result<Option<Json>, OllamaError> {
    let Some(Json::Object(holder)) = fields.get("json_schema") else {
        return Err(OllamaError::Param {
            field: "response_format.json_schema",
        });
    };
    match holder.get("schema") {
        Some(value @ Json::Object(_)) => Ok(Some(value.clone())),
        Some(Json::Null) => Ok(Some(Json::Null)),
        Some(_) | None => Err(OllamaError::Param {
            field: "response_format.json_schema.schema",
        }),
    }
}

fn reasoning_effort(model: &str, value: &Json) -> Result<Option<Json>, OllamaError> {
    match value {
        Json::Null => Ok(None),
        Json::Text(text) if model.starts_with(GPT_OSS_PREFIX) => Ok(Some(Json::Text(text.clone()))),
        Json::Text(text) => Ok(Some(Json::Bool(REASONING_EFFORTS.contains(&text.as_str())))),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(OllamaError::Param {
                field: "reasoning_effort",
            })
        }
    }
}

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    match api_base {
        Some(base) if base.ends_with(GENERATE_PATH) => base.to_owned(),
        Some(base) => format!("{base}{GENERATE_PATH}"),
        None => format!("{DEFAULT_API_BASE}{GENERATE_PATH}"),
    }
}

#[must_use]
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: BTreeMap<String, String>,
) -> OllamaError {
    OllamaError::Api {
        status_code,
        message: message.to_owned(),
        headers,
    }
}
