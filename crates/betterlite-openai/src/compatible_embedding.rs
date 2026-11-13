use std::fmt;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use http::Method;
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use crate::compatible_wire::{EndpointType, header_map, validate_environment};
use crate::embedding::EmbeddingResponse;
use crate::openai_base_config::Headers;
use error::EmbeddingError;

pub mod error;
mod tests;

// B158 owns the blocking wrappers, so Sync only leaves out the outer failure log the async handler writes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sync,
    Async,
}

pub struct EmbeddingCall {
    pub model: String,
    pub input: Json,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub options: IndexMap<String, Json>,
    pub headers: Option<Headers>,
    pub custom_endpoint: bool,
    pub timeout: Duration,
    pub mode: Mode,
}

#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingPlan {
    pub url: String,
    pub headers: Headers,
    pub data: IndexMap<String, Json>,
    pub input: Json,
    pub timeout: Duration,
    pub mode: Mode,
}

// the headers carry the bearer token, so Debug names them without their values as OpenAiConfig does
impl fmt::Debug for EmbeddingPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmbeddingPlan")
            .field("url", &self.url)
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("data", &self.data)
            .field("input", &self.input)
            .field("timeout", &self.timeout)
            .field("mode", &self.mode)
            .finish()
    }
}

// options set to None or "" are left out, and the rest override model and input as the dict merge does
#[must_use]
pub fn request_body(
    model: String,
    input: &Json,
    options: IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut data = IndexMap::from([
        ("model".to_owned(), Json::Text(model)),
        ("input".to_owned(), input.clone()),
    ]);
    let kept = options
        .into_iter()
        .filter(|(_, value)| *value != Json::Null && *value != Json::Text(String::new()));
    data.extend(kept);
    data
}

pub fn plan_embedding(
    call: EmbeddingCall,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<EmbeddingPlan, EmbeddingError> {
    let (url, headers) = validate_environment(
        call.api_key.as_ref(),
        call.api_base.as_deref(),
        EndpointType::Embeddings,
        call.headers,
        call.custom_endpoint,
    )?;
    let data = request_body(call.model, &call.input, call.options);
    let logged = PreCall {
        input: call.input.clone(),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(url.clone()),
            headers: None,
            complete_input_dict: Some(RawBody::Object(data.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(state, &logged, env).map_err(EmbeddingError::PreCall)?;
    Ok(EmbeddingPlan {
        url,
        headers,
        data,
        input: call.input,
        timeout: call.timeout,
        mode: call.mode,
    })
}

pub async fn send_embedding(
    client: &ProxiedClient,
    plan: &EmbeddingPlan,
) -> Result<Json, EmbeddingError> {
    let body = serde_json::to_vec(&plan.data).map_err(EmbeddingError::Encode)?;
    let request = OutboundRequest {
        method: Method::POST,
        url: Url::parse(&plan.url).map_err(EmbeddingError::Url)?,
        query: Vec::new(),
        headers: header_map(&plan.headers).map_err(EmbeddingError::Header)?,
        body: RequestBody::Json(body),
        deadline: plan.timeout,
    };
    let response = client
        .send_following(request)
        .await
        .map_err(timeout_or_transport)?
        .buffered(client.limits.body_bytes)
        .await
        .map_err(timeout_or_transport)?;
    if !(200..300).contains(&response.status) {
        let status = response.status;
        let text = String::from_utf8(response.body)
            .map_err(|source| EmbeddingError::StatusBody { status, source })?;
        return Err(EmbeddingError::Status { status, text });
    }
    serde_json::from_slice(&response.body).map_err(EmbeddingError::Reply)
}

const fn timeout_or_transport(error: TransportError) -> EmbeddingError {
    if matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    ) {
        EmbeddingError::Timeout(error)
    } else {
        EmbeddingError::Transport(error)
    }
}

fn log_post_call(
    plan: &EmbeddingPlan,
    original_response: Json,
    with_data: bool,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) {
    let call = PostCall {
        original_response,
        input: plan.input.clone(),
        additional_args: AdditionalArgs {
            api_base: None,
            headers: None,
            complete_input_dict: with_data.then(|| RawBody::Object(plan.data.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    post_call(state, &call, env);
}

pub fn finish_embedding(
    plan: &EmbeddingPlan,
    sent: Result<Json, EmbeddingError>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<EmbeddingResponse, EmbeddingError> {
    let outcome = sent.and_then(|reply| {
        log_post_call(plan, reply.clone(), true, state, env);
        serde_json::to_value(&reply)
            .and_then(serde_json::from_value)
            .map_err(EmbeddingError::Response)
    });
    if plan.mode == Mode::Async
        && let Err(error) = &outcome
    {
        log_post_call(plan, Json::Text(error.to_string()), false, state, env);
    }
    outcome
}
