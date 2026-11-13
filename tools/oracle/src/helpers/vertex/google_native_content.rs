mod tests;

use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::vertex_schema_shaping::Raw;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::google_native_content::capabilities::NativeProvider;
use betterlite_vertex::google_native_content::map::{
    studio_optional_params, vertex_optional_params,
};
use betterlite_vertex::google_native_content::request::{
    GenerateContentRequest, transform_request,
};
use betterlite_vertex::google_native_content::response::{RawResponse, transform_response};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    OptionalParams(Box<OptionalParamsCall>),
    Request(Box<RequestCall>),
    Response(Box<ResponseCall>),
}

// the case carries a model for the Python config, which the mapping itself ignores
#[derive(Debug, Deserialize)]
struct OptionalParamsCall {
    provider: Provider,
    config: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    provider: Provider,
    model: String,
    contents: Json,
    #[serde(default)]
    generation_config: Object,
    #[serde(default)]
    settings: BTreeMap<String, String>,
    tools: Option<Json>,
    system_instruction: Option<Json>,
}

// the response transform is inherited from the Studio config by both providers, so the case's provider and model select only the Python instance
#[derive(Debug, Deserialize)]
struct ResponseCall {
    status: u16,
    body: String,
    #[serde(default)]
    headers: IndexMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Provider {
    Studio,
    Vertex,
}

impl Provider {
    const fn native(&self) -> NativeProvider {
        match self {
            Self::Studio => NativeProvider::Studio,
            Self::Vertex => NativeProvider::Vertex,
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let calls = input
            .calls
            .iter()
            .map(resolve)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn resolve(call: &Call) -> HelperResult {
    match call {
        Call::OptionalParams(given) => {
            let mapped = match given.provider {
                Provider::Studio => studio_optional_params(&given.config),
                Provider::Vertex => vertex_optional_params(&given.config),
            };
            Ok(json!({ "config": mapped }))
        }
        Call::Request(given) => request_body(given),
        Call::Response(given) => response_record(given),
    }
}

fn request_body(call: &RequestCall) -> HelperResult {
    let settings = Raw::default().convert(&call.settings).or_config_error()?;
    let request = GenerateContentRequest {
        model: call.model.clone(),
        contents: call.contents.clone(),
        tools: call.tools.clone(),
        generation_config: call.generation_config.clone(),
        system_instruction: call.system_instruction.clone(),
    };
    let body = transform_request(call.provider.native(), &request, &settings).or_value_error()?;
    Ok(json!(body))
}

fn response_record(call: &ResponseCall) -> HelperResult {
    let raw = RawResponse {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
    };
    let native = transform_response(&raw).or_value_error()?;
    Ok(json!(native.response))
}
