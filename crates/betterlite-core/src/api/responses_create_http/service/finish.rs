use std::str::from_utf8;
use std::sync::Arc;

use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_events::{RawResponse, ReplyBody, ResponsesReply};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use http::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::api::responses_agentic_execution::dispatch::call_agentic_completion_hooks;
use crate::api::responses_agentic_execution::executor::{CompletionCall, ContinuationExecutor};
use crate::api::responses_agentic_execution::{AgenticResult, LoggingView};
use crate::api::responses_compact_http::error::ConfigFailure;
use crate::api::responses_compact_http::service::ClientKind;
use crate::api::responses_compact_http::service::facts::{Sent, transport_facts};
use crate::api::responses_hook_policy::callbacks::{PolicyCallback, has_agentic_completion_hook};
use crate::api::responses_hook_policy::safety::ApiSurface;
use crate::api::responses_stream_hooks::cache::{converted_stream_requested, value_is_set};
use crate::api::vector_store_http_write::error::{header_text, status_facts};

use super::super::config::CreateConfig;
use super::super::convert::raw_value;
use super::super::error::{CreateError, Stage, provider_error};
use super::hook::Applied;
use super::{CreateOutcome, CreateSynthetic, LogContext, SentReply, StreamSetup};

// The send catch: a transport failure and any status outside 2xx go through _handle_error
pub(super) fn checked<C>(
    reply: Result<SentReply, TransportError>,
    stream: bool,
    sent: &Sent,
    config: &C,
) -> Result<SentReply, CreateError>
where
    C: CreateConfig,
{
    let answer = reply.map_err(|error| {
        CreateError::Provider(provider_error(&transport_facts(&error, sent), config))
    })?;
    match (stream, answer) {
        (true, SentReply::Streaming(response)) => Ok(SentReply::Streaming(response)),
        (false, SentReply::Buffered(response)) => {
            if !(200..=299).contains(&response.status) {
                return Err(CreateError::Provider(provider_error(
                    &status_facts(&response),
                    config,
                )));
            }
            Ok(SentReply::Buffered(response))
        }
        (true, SentReply::Buffered(_)) | (false, SentReply::Streaming(_)) => {
            Err(CreateError::StreamContract)
        }
    }
}

// The buffered reply reaches transform_response_api_response after the send catch, as the source does
pub(super) fn transformed<C>(
    config: &C,
    answer: UpstreamResponse,
    log: &mut LogContext<'_, '_>,
) -> Result<ResponsesReply, CreateError>
where
    C: CreateConfig,
{
    let headers = response_headers(&answer.headers);
    let text = from_utf8(&answer.body).map_err(|source| CreateError::Step {
        stage: Stage::Response,
        source: ConfigFailure::new(source),
    })?;
    let raw = RawResponse {
        status: answer.status,
        headers: &headers,
        text,
    };
    config
        .transform_response(&raw, log.state, log.env)
        .map_err(|source| CreateError::Step {
            stage: Stage::Response,
            source,
        })
}

// The mock iterator transforms a complete response again; only a typed response can become synthetic events
pub(super) fn typed(body: ReplyBody) -> Result<ResponsesApiResponse, CreateError> {
    match body {
        ReplyBody::Typed(response) => Ok(*response),
        ReplyBody::Lax(_) => Err(CreateError::NotTyped),
    }
}

// Post-agentic processing: sync only when B2419 detects an overridden hook, async always
pub(super) async fn finish<C, E>(
    config: &C,
    mode: ClientKind,
    applied: &Applied,
    callbacks: &[PolicyCallback],
    logging: Option<LoggingView<'_>>,
    executor: &E,
    body: ReplyBody,
    log: &mut LogContext<'_, '_>,
) -> Result<CreateOutcome, CreateError>
where
    C: CreateConfig,
    E: ContinuationExecutor + ?Sized,
{
    let initial = initial_raw(body)?;
    if mode == ClientKind::Sync && !has_agentic_completion_hook(callbacks) {
        return Ok(CreateOutcome::Value(initial));
    }
    let optional: JsonMap = optional_raw(&applied.optional_params)?;
    let messages = followup_messages(&applied.input)?;
    let kwargs = applied.params.to_map();
    let call = CompletionCall {
        response: &initial,
        model: &applied.model,
        messages: &messages,
        optional_params: &optional,
        logging,
        stream: false,
        custom_llm_provider: &applied.custom_llm_provider,
        kwargs: &kwargs,
        api_surface: ApiSurface::Responses,
    };
    let agentic = agentic_callbacks(callbacks);
    let result = match call_agentic_completion_hooks(&call, &agentic, executor).await? {
        Some(AgenticResult::Response(raw)) => raw,
        Some(AgenticResult::Stream(_)) => return Err(CreateError::AgenticStream),
        None => initial,
    };
    if !converted_stream_requested(&kwargs) || value_is_set(kwargs.get("_agentic_loop_depth")) {
        return Ok(CreateOutcome::Value(result));
    }
    wrap(config, applied, &result, log)
}

// _wrap_responses_response_as_fake_stream: no request_data, litellm_metadata or call_type here
pub(super) fn wrap<C>(
    config: &C,
    applied: &Applied,
    result: &RawValue,
    log: &mut LogContext<'_, '_>,
) -> Result<CreateOutcome, CreateError>
where
    C: CreateConfig,
{
    let headers = IndexMap::new();
    let raw = RawResponse {
        status: 200,
        headers: &headers,
        text: result.get(),
    };
    let reply = config
        .transform_response(&raw, log.state, log.env)
        .map_err(|source| CreateError::Step {
            stage: Stage::Response,
            source,
        })?;
    let response = typed(reply.response)?;
    Ok(CreateOutcome::Synthetic(CreateSynthetic {
        setup: StreamSetup {
            model: applied.model.clone(),
            custom_llm_provider: applied.custom_llm_provider.clone(),
            litellm_metadata: None,
            request_data: None,
            call_type: None,
        },
        response,
    }))
}

#[derive(Serialize)]
struct UserTurn<'turn> {
    role: &'static str,
    content: &'turn str,
}

// messages=(input if isinstance(input, list) else [{"role": "user", "content": input}])
fn followup_messages(input: &ResponsesInput) -> Result<Vec<FollowupMessage>, CreateError> {
    match input {
        ResponsesInput::Text(text) => Ok(vec![FollowupMessage(raw_value(&UserTurn {
            role: "user",
            content: text,
        })?)]),
        ResponsesInput::Items(items) => items
            .iter()
            .map(|item| Ok(FollowupMessage(raw_value(&item.0)?)))
            .collect(),
    }
}

fn optional_raw(optional: &Object) -> Result<JsonMap, CreateError> {
    optional
        .iter()
        .map(|(key, value)| Ok((key.clone(), raw_value(value)?)))
        .collect()
}

fn initial_raw(body: ReplyBody) -> Result<Box<RawValue>, CreateError> {
    match body {
        ReplyBody::Typed(response) => raw_value(&*response),
        ReplyBody::Lax(fields) => raw_value(&Json::Object(fields)),
    }
}

fn agentic_callbacks(callbacks: &[PolicyCallback]) -> Vec<Arc<dyn AgenticCallback>> {
    callbacks
        .iter()
        .filter_map(|callback| callback.agentic.clone())
        .collect()
}

fn response_headers(headers: &HttpHeaders) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect()
}
