use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Number;

use super::error::OllamaError;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 13] = [
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "top_p",
    "temperature",
    "seed",
    "frequency_penalty",
    "stop",
    "tools",
    "tool_choice",
    "functions",
    "response_format",
    "reasoning_effort",
];

const DEFAULT_API_BASE: &str = "http://localhost:11434";
const CHAT_PATH: &str = "/api/chat";
const AUTHORIZATION: &str = "Authorization";
const JSON_OBJECT: &str = "json_object";
const JSON_SCHEMA: &str = "json_schema";
const GPT_OSS_PREFIX: &str = "gpt-oss";
const REASONING_EFFORTS: [&str; 3] = ["low", "medium", "high"];

// the deployment-level Ollama options LiteLLM keeps as class attributes and merges into a request's options
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OllamaChatConfig {
    #[serde(default)]
    pub mirostat: Option<Number>,
    #[serde(default)]
    pub mirostat_eta: Option<Number>,
    #[serde(default)]
    pub mirostat_tau: Option<Number>,
    #[serde(default)]
    pub num_ctx: Option<Number>,
    #[serde(default)]
    pub num_gqa: Option<Number>,
    #[serde(default)]
    pub num_thread: Option<Number>,
    #[serde(default)]
    pub repeat_last_n: Option<Number>,
    #[serde(default)]
    pub repeat_penalty: Option<Number>,
    #[serde(default)]
    pub seed: Option<Number>,
    #[serde(default)]
    pub tfs_z: Option<Number>,
    #[serde(default)]
    pub num_predict: Option<Number>,
    #[serde(default)]
    pub top_k: Option<Number>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub temperature: Option<Number>,
    #[serde(default)]
    pub stop: Option<Vec<String>>,
    #[serde(default)]
    pub top_p: Option<Number>,
}

impl OllamaChatConfig {
    // LiteLLM's get_config keeps the class attributes in definition order, with the undeclared constructor fields appended
    #[must_use]
    pub fn get_config(&self) -> IndexMap<String, Json> {
        let entries: [(&str, Option<Json>); 17] = [
            ("mirostat", self.mirostat.clone().map(Json::Number)),
            ("mirostat_eta", self.mirostat_eta.clone().map(Json::Number)),
            ("mirostat_tau", self.mirostat_tau.clone().map(Json::Number)),
            ("num_ctx", self.num_ctx.clone().map(Json::Number)),
            ("num_gqa", self.num_gqa.clone().map(Json::Number)),
            ("num_thread", self.num_thread.clone().map(Json::Number)),
            (
                "repeat_last_n",
                self.repeat_last_n.clone().map(Json::Number),
            ),
            (
                "repeat_penalty",
                self.repeat_penalty.clone().map(Json::Number),
            ),
            ("seed", self.seed.clone().map(Json::Number)),
            ("tfs_z", self.tfs_z.clone().map(Json::Number)),
            ("num_predict", self.num_predict.clone().map(Json::Number)),
            ("top_k", self.top_k.clone().map(Json::Number)),
            ("system", self.system.clone().map(Json::Text)),
            ("template", self.template.clone().map(Json::Text)),
            ("temperature", self.temperature.clone().map(Json::Number)),
            (
                "stop",
                self.stop
                    .clone()
                    .map(|values| Json::List(values.into_iter().map(Json::Text).collect())),
            ),
            ("top_p", self.top_p.clone().map(Json::Number)),
        ];
        entries
            .into_iter()
            .filter_map(|(name, value)| value.map(|field| (String::from(name), field)))
            .collect()
    }
}

pub fn map_openai_params(
    non_default: &IndexMap<String, Json>,
    optional: &mut IndexMap<String, Json>,
    model: &str,
) -> Result<(), OllamaError> {
    for (param, value) in non_default {
        match param.as_str() {
            "max_tokens" | "max_completion_tokens" => {
                optional.insert(String::from("num_predict"), value.clone());
            }
            "stream" | "temperature" | "seed" | "top_p" | "stop" => {
                optional.insert(param.clone(), value.clone());
            }
            "frequency_penalty" => {
                optional.insert(String::from("repeat_penalty"), value.clone());
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
            // tool_choice causes ollama requests to hang, so LiteLLM never maps it
            "tools" | "functions" => {
                optional.insert(String::from("tools"), value.clone());
            }
            _ => {}
        }
    }
    Ok(())
}

fn response_format(value: &Json) -> Result<Option<Json>, OllamaError> {
    let Json::Object(fields) = value else {
        return Ok(None);
    };
    match fields.get("type") {
        Some(Json::Text(kind)) if kind == JSON_OBJECT => Ok(Some(Json::Text(String::from("json")))),
        Some(Json::Text(kind)) if kind == JSON_SCHEMA => json_schema_format(fields),
        Some(
            Json::Null
            | Json::Text(_)
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => Ok(None),
    }
}

fn json_schema_format(fields: &IndexMap<String, Json>) -> Result<Option<Json>, OllamaError> {
    let Some(Json::Object(schema)) = fields.get(JSON_SCHEMA) else {
        return match fields.get(JSON_SCHEMA) {
            Some(Json::Null) | None => Ok(None),
            Some(_) => Err(OllamaError::Param {
                field: "response_format.json_schema",
            }),
        };
    };
    match schema.get("schema") {
        Some(value @ Json::Object(_)) => Ok(Some(value.clone())),
        Some(Json::Null) | None => Ok(None),
        Some(_) => Err(OllamaError::Param {
            field: "response_format.json_schema.schema",
        }),
    }
}

fn reasoning_effort(model: &str, value: &Json) -> Result<Option<Json>, OllamaError> {
    match value {
        Json::Null => Ok(None),
        Json::Text(text) if model.starts_with(GPT_OSS_PREFIX) => Ok(Some(Json::Text(text.clone()))),
        Json::Text(text) => Ok(Some(Json::Bool(
            REASONING_EFFORTS.iter().any(|effort| effort == text),
        ))),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(OllamaError::Param {
                field: "reasoning_effort",
            })
        }
    }
}

pub fn validate_environment(
    headers: &mut BTreeMap<String, String>,
    api_key: Option<&SecretString>,
) {
    let Some(key) = api_key else {
        return;
    };
    if !headers.contains_key(AUTHORIZATION) {
        headers.insert(
            String::from(AUTHORIZATION),
            format!("Bearer {}", key.expose_secret()),
        );
    }
}

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    match api_base {
        Some(base) if base.ends_with(CHAT_PATH) => base.to_owned(),
        Some(base) => format!("{base}{CHAT_PATH}"),
        None => format!("{DEFAULT_API_BASE}{CHAT_PATH}"),
    }
}
