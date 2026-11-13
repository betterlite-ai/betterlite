use std::collections::BTreeMap;

use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::nvidia_nim_chat::NvidiaNimConfig;
use betterlite_openai::nvidia_nim_embedding::{SUPPORTED_OPENAI_PARAMS, map_params};
use betterlite_openai::openai_base_config::Object;
use betterlite_provider::api::rerank_contracts::Params;
use serde::Deserialize;
use serde_json::{Value, json};

mod passthrough;
mod rerank;

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    kind: String,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    cases: Vec<Case>,
    #[serde(default)]
    urls: Vec<UrlCase>,
    #[serde(default)]
    headers: Vec<Case>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    #[serde(default)]
    pub(crate) config: String,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) non_default: Option<Params>,
    #[serde(default)]
    pub(crate) optional: Option<Params>,
    #[serde(default)]
    pub(crate) kwargs: Option<Params>,
    #[serde(default)]
    pub(crate) top_n: Option<i64>,
    #[serde(default)]
    pub(crate) query: String,
    #[serde(default)]
    pub(crate) documents: Vec<RerankDocument>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) env_key: Option<String>,
    #[serde(default)]
    pub(crate) global_key: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) stream: Option<Value>,
    #[serde(default)]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) request_data: Params,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCase {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) env_base: Option<String>,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) model_group: Option<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.kind.as_str() {
            "chat_supported" => Ok(json!({"supported": chat_supported(&input.models)})),
            "chat_map" => Ok(json!({"mapped": chat_map(&input.cases)})),
            "embedding_map" => Ok(embedding_map(&input.cases)),
            "rerank_select" => Ok(rerank::select(input)),
            "rerank_params" => Ok(rerank::params(&input.cases, &input.headers)),
            "rerank_request" => Ok(rerank::request(&input.cases)),
            "rerank_response" => Ok(rerank::response(&input.cases)),
            "passthrough_url" => Ok(passthrough::urls(&input.urls)),
            "passthrough_credentials" => Ok(passthrough::credentials(&input.cases)),
            _ => Self::not_built(&input.kind),
        }
    }
}

fn chat_supported(models: &[String]) -> Vec<Vec<&'static str>> {
    models
        .iter()
        .map(|model| NvidiaNimConfig::supported_params(model))
        .collect()
}

fn chat_map(cases: &[Case]) -> Vec<Params> {
    let empty = Params::new();
    cases
        .iter()
        .map(|case| {
            NvidiaNimConfig::map_params(
                &case.model,
                case.non_default.as_ref().unwrap_or(&empty),
                case.optional.clone().unwrap_or_default(),
            )
        })
        .collect()
}

fn embedding_map(cases: &[Case]) -> Value {
    let mapped: Vec<Value> = cases
        .iter()
        .map(|case| {
            let non_default = case.non_default.clone().unwrap_or_default();
            let optional = case.optional.clone().unwrap_or_default();
            match map_params(&non_default, optional, case.kwargs.as_ref()) {
                Ok(params) => json!({"ok": true, "mapped": params}),
                Err(_) => json!({"ok": false}),
            }
        })
        .collect();
    json!({"supported": SUPPORTED_OPENAI_PARAMS, "mapped": mapped})
}

pub(crate) fn object(value: Value) -> Object {
    match value {
        Value::Object(fields) => fields,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Object::new()
        }
    }
}
