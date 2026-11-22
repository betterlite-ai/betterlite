use std::collections::BTreeMap;

use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::bedrock_settings::ErrorHeaders;
use betterlite_bedrock::invoke_agent::error::InvokeAgentRequestError;
use betterlite_bedrock::invoke_agent::payload::transform_request;
use betterlite_bedrock::invoke_agent::policy::{
    SHOULD_FAKE_STREAM, SUPPORTED_OPENAI_PARAMS, get_error_class,
};
use betterlite_bedrock::invoke_agent::url::{CompleteUrl, InvokeEnvironment, complete_url};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

mod signing;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(run_scenario(input))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "scenario", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    AgentIdAliasPath {
        runs: Vec<UrlRun>,
    },
    SessionId {
        runs: Vec<UrlRun>,
    },
    AgentInputText {
        runs: Vec<TransformRun>,
    },
    AgentSigningAndCapability {
        signing: Vec<SignRun>,
        policy: PolicyRun,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlRun {
    model: String,
    api_base: Option<String>,
    #[serde(default)]
    optional_params: ParamMap,
    #[serde(default)]
    environment: EnvironmentRun,
    uuid: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentRun {
    runtime_endpoint: Option<String>,
    region_name: Option<String>,
    region: Option<String>,
    session_region: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformRun {
    messages: Vec<Message>,
    #[serde(default)]
    optional_params: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignRun {
    model: String,
    api_base: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    optional_params: ParamMap,
    request_data: IndexMap<String, Json>,
    api_key: Option<String>,
    configured_bearer: Option<String>,
    credentials: CredentialRun,
    now: u64,
    stream: Option<bool>,
    fake_stream: Option<bool>,
    #[serde(default)]
    environment: EnvironmentRun,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialRun {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolicyRun {
    optional_params: ParamMap,
    headers: Vec<(String, String)>,
    error: ErrorRun,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorRun {
    status: u16,
    message: String,
    headers: Vec<(String, String)>,
}

fn run_scenario(input: &Input) -> Value {
    match input {
        Input::AgentIdAliasPath { runs } | Input::SessionId { runs } => json!({
            "results": runs.iter().map(url_result).collect::<Vec<_>>(),
        }),
        Input::AgentInputText { runs } => json!({
            "results": runs.iter().map(transform_result).collect::<Vec<_>>(),
        }),
        Input::AgentSigningAndCapability { signing, policy } => json!({
            "signing": signing.iter().map(signing::sign_result).collect::<Vec<_>>(),
            "policy": policy_result(policy),
        }),
    }
}

fn url_result(run: &UrlRun) -> Value {
    let Ok(uuid) = Uuid::parse_str(&run.uuid) else {
        return json!({"ok": false, "error": "invalid_uuid", "params": run.optional_params});
    };
    match complete_url(CompleteUrl {
        api_base: run.api_base.as_deref(),
        model: &run.model,
        optional_params: &run.optional_params,
        environment: environment_of(&run.environment),
        generated_session_id: uuid,
    }) {
        Ok(url) => json!({"ok": true, "url": url, "params": run.optional_params}),
        Err(error) => json!({
            "ok": false,
            "error": error_kind(&error),
            "params": run.optional_params,
        }),
    }
}

fn transform_result(run: &TransformRun) -> Value {
    match transform_request(&run.messages, &run.optional_params) {
        Ok(payload) => json!({"ok": true, "payload": payload}),
        Err(error) => json!({"ok": false, "error": error_kind(&error)}),
    }
}

fn policy_result(run: &PolicyRun) -> Value {
    let headers = ErrorHeaders::Http(run.error.headers.clone());
    let error = match get_error_class(run.error.message.clone(), run.error.status, Some(headers)) {
        Ok(error) => json!({
            "status_code": error.error.status_code,
            "status_code_is_synthesized": error.error.status_code_is_synthesized,
            "headers": header_map(&error.response_headers),
            "response_status": error.error.response.status_code,
        }),
        Err(error) => json!({"class": format!("{error:?}")}),
    };
    json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": run.optional_params,
        "should_fake_stream": [SHOULD_FAKE_STREAM, SHOULD_FAKE_STREAM, SHOULD_FAKE_STREAM],
        "headers": header_map(&run.headers),
        "error": error,
    })
}

pub(super) fn environment_of(run: &EnvironmentRun) -> InvokeEnvironment<'_> {
    InvokeEnvironment {
        runtime_endpoint: run.runtime_endpoint.as_deref(),
        region_inputs: RegionInputs {
            environment_region_name: run.region_name.as_deref(),
            environment_region: run.region.as_deref(),
            session_region: run.session_region.as_deref(),
        },
    }
}

pub(super) fn error_kind(error: &InvokeAgentRequestError) -> String {
    match error {
        InvokeAgentRequestError::InvalidModelFormat => "invalid_model_format".to_owned(),
        InvokeAgentRequestError::EmptyMessages => "empty_messages".to_owned(),
        InvokeAgentRequestError::WrongType { field } => format!("wrong_type:{field}"),
        InvokeAgentRequestError::Segment(source) => segment_kind(source),
        InvokeAgentRequestError::Endpoint(source) => endpoint_kind(source),
        InvokeAgentRequestError::Parameters { .. } => "parameters".to_owned(),
        InvokeAgentRequestError::Signing { .. } => "signing".to_owned(),
    }
}

fn segment_kind(error: &HttpError) -> String {
    match error {
        HttpError::Required { field_name } => format!("required:{field_name}"),
        HttpError::DotSegment { field_name } => format!("dot_segment:{field_name}"),
        HttpError::UnsupportedProxyScheme { .. } => "unsupported_proxy_scheme".to_owned(),
        HttpError::InvalidProxyUrl { .. } => "invalid_proxy_url".to_owned(),
        HttpError::Client(_) => "client".to_owned(),
        HttpError::Request(_) => "request".to_owned(),
        HttpError::Timeout => "timeout".to_owned(),
        HttpError::TooLarge { .. } => "too_large".to_owned(),
    }
}

fn endpoint_kind(error: &AwsEndpointError) -> String {
    match error {
        AwsEndpointError::Region { .. } => "invalid_region".to_owned(),
        AwsEndpointError::Endpoint { .. } => "invalid_endpoint".to_owned(),
        AwsEndpointError::Pattern(_) => "invalid_pattern".to_owned(),
        AwsEndpointError::Field { .. } => "invalid_field".to_owned(),
    }
}

pub(super) fn optional_headers(pairs: &[(String, String)]) -> Vec<(String, Option<String>)> {
    pairs
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect()
}

pub(super) fn header_map(pairs: &[(String, String)]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (name, value) in pairs {
        map.insert(name.clone(), value.clone());
    }
    map
}
