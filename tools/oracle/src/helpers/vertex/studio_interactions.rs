use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_vertex::studio_interactions::credentials::{StudioParams, validate_environment};
use betterlite_vertex::studio_interactions::decode::{
    RawResponse, decode_studio_interaction, transform_delete_response, transform_get_response,
    transform_response, transform_streaming_response,
};
use betterlite_vertex::studio_interactions::error::StudioInteractionError;
use betterlite_vertex::studio_interactions::records::CancelInteractionResult;
use betterlite_vertex::studio_interactions::request::{InteractionOptions, transform_request};
use betterlite_vertex::studio_interactions::urls::{Lifecycle, create_url, lifecycle_request};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

type Failure = Box<HelperError>;
type Env = BTreeMap<String, String>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    TransformRequest(Box<RequestCall>),
    ValidateEnvironment(EnvironmentCall),
    CreateUrl(UrlCall),
    Lifecycle(LifecycleCall),
    Response(ResponseCall),
    Delete(DeleteCall),
    StreamChunk(ChunkCall),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    model: Option<String>,
    agent: Option<String>,
    input: Option<String>,
    optional_params: InteractionOptions,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentCall {
    #[serde(default)]
    headers: IndexMap<String, String>,
    api_key: Option<String>,
    with_params: bool,
    #[serde(default)]
    env: Env,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    api_base: Option<String>,
    api_key: Option<String>,
    stream: Option<bool>,
    #[serde(default)]
    env: Env,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Get,
    Delete,
    Cancel,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleCall {
    action: Action,
    interaction_id: String,
    api_base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    env: Env,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reply {
    Create,
    Get,
    Cancel,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseCall {
    action: Reply,
    status: u16,
    #[serde(default)]
    headers: IndexMap<String, String>,
    body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteCall {
    status: u16,
    #[serde(default)]
    headers: IndexMap<String, String>,
    body: String,
    interaction_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkCall {
    chunk: Json,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let calls = input
            .calls
            .iter()
            .map(resolve)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn failure(error: &StudioInteractionError) -> Failure {
    Box::new(HelperError {
        class: "StudioInteractionError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn settings(env: &Env) -> Settings {
    let environment_key = env
        .get("GOOGLE_API_KEY")
        .filter(|value| !value.is_empty())
        .or_else(|| env.get("GEMINI_API_KEY").filter(|value| !value.is_empty()))
        .map(|value| SecretString::from(value.as_str()));
    Settings {
        environment_key,
        generic_api_key: None,
        api_base: env
            .get("GEMINI_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
    }
}

fn params(api_key: Option<&String>) -> StudioParams {
    StudioParams {
        api_key: api_key.map(|key| SecretString::from(key.as_str())),
    }
}

// httpx lowercases header names, and LiteLLM reads them back through dict(raw_response.headers).
fn lowercased(headers: &IndexMap<String, String>) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect()
}

fn resolve(call: &Call) -> Result<Value, Failure> {
    match call {
        Call::TransformRequest(request) => request_call(request),
        Call::ValidateEnvironment(given) => Ok(environment_call(given)),
        Call::CreateUrl(given) => url_call(given),
        Call::Lifecycle(given) => lifecycle_call(given),
        Call::Response(given) => response_call(given),
        Call::Delete(given) => delete_call(given),
        Call::StreamChunk(given) => {
            let record =
                transform_streaming_response(&given.chunk).map_err(|error| failure(&error))?;
            Ok(json!({ "record": record }))
        }
    }
}

fn request_call(call: &RequestCall) -> Result<Value, Failure> {
    let text = call.input.as_ref().map(|text| Json::Text(text.clone()));
    let body = transform_request(
        call.model.as_deref(),
        call.agent.as_deref(),
        text.as_ref(),
        &call.optional_params,
    )
    .map_err(|error| failure(&error))?;
    Ok(json!({ "body": body }))
}

fn environment_call(call: &EnvironmentCall) -> Value {
    let given = params(call.api_key.as_ref());
    let chosen = call.with_params.then_some(&given);
    let result = validate_environment(&call.headers, chosen, &settings(&call.env));
    let revealed = result
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect::<IndexMap<_, _>>();
    json!({ "headers": revealed })
}

fn url_call(call: &UrlCall) -> Result<Value, Failure> {
    let url = create_url(
        call.api_base.as_deref(),
        &params(call.api_key.as_ref()),
        call.stream.unwrap_or(false),
        &settings(&call.env),
    )
    .map_err(|error| failure(&error))?;
    Ok(json!({ "url": url }))
}

fn lifecycle_call(call: &LifecycleCall) -> Result<Value, Failure> {
    let kind = match call.action {
        Action::Get => Lifecycle::Get,
        Action::Delete => Lifecycle::Delete,
        Action::Cancel => Lifecycle::Cancel,
    };
    let request = lifecycle_request(
        kind,
        &call.interaction_id,
        call.api_base.as_deref(),
        &params(call.api_key.as_ref()),
        &settings(&call.env),
    )
    .map_err(|error| failure(&error))?;
    Ok(json!({ "url": request.url, "body": request.body }))
}

fn delete_call(call: &DeleteCall) -> Result<Value, Failure> {
    let headers = lowercased(&call.headers);
    let response = RawResponse {
        status: call.status,
        headers: &headers,
        body: call.body.as_bytes(),
    };
    let result = transform_delete_response(&response, &call.interaction_id)
        .map_err(|error| failure(&error))?;
    Ok(json!(result))
}

fn response_call(call: &ResponseCall) -> Result<Value, Failure> {
    let headers = lowercased(&call.headers);
    let response = RawResponse {
        status: call.status,
        headers: &headers,
        body: call.body.as_bytes(),
    };
    let interaction = match call.action {
        Reply::Cancel => {
            let result = decode_studio_interaction::<CancelInteractionResult>(&response)
                .map_err(|error| failure(&error))?;
            return Ok(json!({ "record": result }));
        }
        Reply::Create => transform_response(&response, |raw| {
            process_response_headers(HeaderSource::Upstream(raw))
        }),
        Reply::Get => transform_get_response(&response),
    }
    .map_err(|error| failure(&error))?;
    Ok(json!({
        "record": interaction.record,
        "headers": interaction.hidden_params.headers,
        "additional_headers": interaction.hidden_params.additional_headers,
    }))
}

mod tests;
