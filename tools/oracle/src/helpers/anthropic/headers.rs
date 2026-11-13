use std::collections::{BTreeMap, BTreeSet};

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_anthropic::headers::HeadersError;
use betterlite_anthropic::headers::catalog::{
    ModelRecord, create_anthropic_model_list_response, model_created_at,
};
use betterlite_anthropic::headers::environment::{
    EnvironmentRequest, HeaderLiteLlmParams, HeaderParams, validate_environment,
};
use betterlite_anthropic::headers::request::{
    HeaderFeature, HeaderFeatures, HeaderRequest, get_anthropic_headers,
};
use betterlite_anthropic::headers::response::process_anthropic_headers;
use betterlite_config::sections::anthropic_credentials::Settings as CredentialSettings;
use betterlite_config::sections::anthropic_headers::Settings as HeaderSettings;
use betterlite_prompts::arguments::json::Json;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

const BETA_HEADER: &str = "anthropic-beta";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadersCall {
    api_key: Option<String>,
    auth_token: Option<String>,
    anthropic_version: Option<String>,
    api_base: Option<String>,
    use_bearer_for_custom_base: bool,
    user_betas: Option<Vec<String>>,
    features: Vec<HeaderFeature>,
    computer_tool_used: Option<String>,
    is_vertex_request: bool,
    web_search_tool_used: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentCall {
    model: String,
    headers: Vec<Header>,
    api_key: Option<String>,
    api_base: Option<String>,
    litellm_params: HeaderLiteLlmParams,
    messages: Vec<Json>,
    optional_params: HeaderParams,
    effort_used: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelListCall {
    models: Vec<ModelRecord>,
    display_names: BTreeMap<String, String>,
    listed_ids: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseHeadersCall {
    headers: Vec<Header>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Headers(HeadersCall),
    Environment(EnvironmentCall),
    ModelList(ModelListCall),
    ResponseHeaders(ResponseHeadersCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    environment: BTreeMap<String, String>,
    calls: Vec<Call>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let credentials =
            CredentialSettings::resolve_environment(&input.environment).or_config_error()?;
        let settings = AnthropicSettings {
            api_key: credentials.api_key,
            auth_token: credentials.auth_token,
            api_base: credentials.api_base,
            use_bearer_for_custom_base: false,
            anthropic_version: ANTHROPIC_VERSION.to_owned(),
        };
        let loaded = HeaderSettings::resolve_environment(&input.environment).or_config_error()?;
        let created_at = model_created_at(loaded.default_model_created_at_time)
            .map_err(|error| failure(&error))?;
        let mut answers = Vec::new();
        for call in &input.calls {
            answers.push(match call {
                Call::Headers(headers) => headers_answer(headers),
                Call::Environment(environment) => environment_answer(environment, &settings)?,
                Call::ModelList(list) => model_list_answer(list, &created_at)?,
                Call::ResponseHeaders(response) => response_headers_answer(response)?,
            });
        }
        Ok(json!({ "calls": answers }))
    }
}

fn secret(value: Option<&String>) -> Option<SecretString> {
    value.cloned().map(SecretString::from)
}

// the anthropic-beta list is built from a Python set, so only its sorted unique tokens are compared
fn project_headers(headers: &IndexMap<String, String>) -> Value {
    let mut projected = serde_json::Map::new();
    for (name, value) in headers {
        let projection = if name == BETA_HEADER {
            let tokens: BTreeSet<&str> = value.split(',').collect();
            json!(tokens)
        } else {
            Value::String(value.clone())
        };
        projected.insert(name.clone(), projection);
    }
    json!({ "headers": projected })
}

fn headers_answer(call: &HeadersCall) -> Value {
    let api_key = secret(call.api_key.as_ref());
    let auth_token = secret(call.auth_token.as_ref());
    let features = HeaderFeatures {
        features: call.features.iter().copied().collect(),
        computer_tool_used: call.computer_tool_used.clone(),
        is_vertex_request: call.is_vertex_request,
        web_search_tool_used: call.web_search_tool_used,
    };
    let headers = get_anthropic_headers(&HeaderRequest {
        api_key: api_key.as_ref(),
        auth_token: auth_token.as_ref(),
        anthropic_version: call.anthropic_version.as_deref(),
        api_base: call.api_base.as_deref(),
        use_bearer_for_custom_base: call.use_bearer_for_custom_base,
        user_betas: call.user_betas.as_deref(),
        features: &features,
    });
    project_headers(&headers.values)
}

fn environment_answer(call: &EnvironmentCall, settings: &AnthropicSettings) -> HelperResult {
    let caller_headers: IndexMap<String, String> = call
        .headers
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect();
    let api_key = secret(call.api_key.as_ref());
    let headers = validate_environment(
        &EnvironmentRequest {
            model: &call.model,
            caller_headers: &caller_headers,
            api_key: api_key.as_ref(),
            api_base: call.api_base.as_deref(),
            litellm_params: &call.litellm_params,
            messages: &call.messages,
            optional_params: &call.optional_params,
            effort_used: call.effort_used,
        },
        settings,
    )
    .map_err(|error| failure(&error))?;
    Ok(project_headers(&headers.values))
}

fn model_list_answer(call: &ModelListCall, created_at: &str) -> HelperResult {
    let response = create_anthropic_model_list_response(
        &call.models,
        &call.display_names,
        &call.listed_ids,
        created_at,
    );
    let response = serde_json::to_value(&response).or_config_error()?;
    Ok(json!({ "response": response }))
}

fn response_headers_answer(call: &ResponseHeadersCall) -> HelperResult {
    let mut headers = HeaderMap::new();
    for header in &call.headers {
        let name = HeaderName::from_bytes(header.name.as_bytes()).or_config_error()?;
        let value = HeaderValue::from_str(&header.value).or_config_error()?;
        headers.append(name, value);
    }
    Ok(project_headers(&process_anthropic_headers(&headers)))
}

fn failure(error: &HeadersError) -> Box<HelperError> {
    let class = match error {
        HeadersError::Authentication { .. } => "AuthenticationError",
        HeadersError::Features(_) | HeadersError::Timestamp { .. } => "Error",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: error.status_code(),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
