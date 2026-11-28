use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogLevel, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use betterlite_vertex::partner_response_config::{
    PartnerResponseCall, PartnerResponseInput, partner_response_config,
};
use betterlite_vertex::vertex_relay_metadata::payload::{
    ResponseLoggingPayloadInput, response_logging_payload,
};
use betterlite_vertex::vertex_relay_metadata::types::{
    CompletionResponse, CustomLlmProvider, ResponseLoggingKwargs, VertexPublisher,
};
use betterlite_vertex::vertex_relay_metadata::url_models::{
    extract_model_from_url, publisher_from_url,
};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;
use uuid::Uuid;

use super::counter::PartnerTools;
use super::response_body::{empty_chat_response, set_body_id};
use super::{VertexPassthroughError, VertexPassthroughInput};
use crate::pass_through_logging::gemini_passthrough_logging::cost::{
    FactoryCost, PassthroughCompletionCost,
};
use crate::pass_through_logging::gemini_passthrough_logging::relay::{RelayLogging, relay_kwargs};

// The partner transform writes its work to the debug log, which never reaches the client.
struct DebugSink;

impl LogSink for DebugSink {
    fn emit(&self, record: LogRecord) {
        match record.level {
            LogLevel::Debug => tracing::debug!("{}", record.message),
            LogLevel::Warning => tracing::warn!("{}", record.message),
            LogLevel::Error => tracing::error!("{}", record.message),
        }
    }
}

// The fields the partner transform reads from the reply and the call it logs to.
struct PartnerReply<'reply> {
    model: &'reply str,
    status: u16,
    headers: &'reply HeaderMap,
    body: &'reply [u8],
    now: DateTime<FixedOffset>,
    call_log: &'reply mut LoggingState,
}

// The rawPredict reply: the partner transform when a publisher owns the model, else an empty chat response.
pub(super) fn raw_predict(
    cost: &dyn PassthroughCompletionCost,
    tools: Option<&PartnerTools>,
    input: VertexPassthroughInput<'_>,
    vertex_location: Option<&str>,
) -> Result<HandlerOutcome, VertexPassthroughError> {
    let Some(partner) = tools else {
        return Err(VertexPassthroughError::Tools);
    };
    let kwargs = input.kwargs;
    let model = extract_model_from_url(input.url_route);
    let publisher = publisher_from_url(input.url_route);
    // LiteLLM decodes the reply before it branches on the publisher, so a body that is not JSON fails either way.
    serde_json::from_slice::<Json>(input.response_body)
        .map_err(|source| VertexPassthroughError::RawBody { source })?;
    let mut relay = RelayLogging::project(input.logging_obj);
    let mut body = match publisher {
        Some(found) => partner_body(
            partner,
            found,
            PartnerReply {
                model: &model,
                status: input.status.as_u16(),
                headers: input.response_headers,
                body: input.response_body,
                now: input.end_time,
                call_log: input.call_log,
            },
        )?,
        None => empty_chat_response(input.end_time.timestamp()),
    };
    let logged = log_reply(cost, &mut relay, &mut body, &model, vertex_location)?;
    relay.commit();
    Ok(HandlerOutcome {
        result: Some(PassThroughResponse::ModelResponse(body)),
        kwargs: relay_kwargs(&kwargs, logged),
    })
}

fn partner_body(
    partner: &PartnerTools,
    publisher: VertexPublisher,
    reply: PartnerReply<'_>,
) -> Result<Json, VertexPassthroughError> {
    let PartnerReply {
        model,
        status,
        headers,
        body,
        now,
        call_log,
    } = reply;
    let sink = DebugSink;
    let env = partner_env(&sink, &partner.redactor, now);
    let messages = dummy_messages();
    let request_data = IndexMap::new();
    let optional_params = IndexMap::new();
    let litellm_params = IndexMap::new();
    // ModelResponse() starts with a chatcmpl id, which the payload step replaces.
    let starting_id = format!("chatcmpl-{}", call_log.litellm_call_id);
    partner_response_config(publisher)
        .transform_response(PartnerResponseCall {
            input: PartnerResponseInput {
                model,
                starting_id: &starting_id,
                status,
                headers,
                body,
                messages: &messages,
                request_data: &request_data,
                optional_params: &optional_params,
                litellm_params: &litellm_params,
                json_mode: None,
                now: now.timestamp(),
                identifier: Uuid::new_v4,
                disable_add_prefix_to_prompt: false,
                state: call_log,
                env: &env,
            },
            counter: &partner.counter,
        })
        .map_err(|source| VertexPassthroughError::Partner {
            publisher: publisher.as_str(),
            source: Box::new(source),
        })
}

fn partner_env<'env>(
    sink: &'env DebugSink,
    redactor: &'env Redactor,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

// Set the logged id, drop the transform's private params and price the reply, all before the relay commit.
fn log_reply(
    cost: &dyn PassthroughCompletionCost,
    relay: &mut RelayLogging<'_>,
    body: &mut Json,
    model: &str,
    vertex_location: Option<&str>,
) -> Result<ResponseLoggingKwargs, VertexPassthroughError> {
    strip_hidden_params(body);
    set_body_id(body, &relay.logging.litellm_call_id)?;
    let mut completion = completion_response(body);
    let prefixed_model = format!("vertex_ai/{model}");
    Ok(response_logging_payload(
        &FactoryCost(cost),
        ResponseLoggingPayloadInput {
            response: &mut completion,
            model: &prefixed_model,
            kwargs: ResponseLoggingKwargs::default(),
            logging: &mut relay.logging,
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location,
        },
    ))
}

pub(super) fn dummy_messages() -> Vec<Json> {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert(
        "content".to_owned(),
        Json::Text("no-message-pass-through-endpoint".to_owned()),
    );
    vec![Json::Object(message)]
}

// model_dump() leaves the partner transform's private hidden params out of the logged reply.
fn strip_hidden_params(body: &mut Json) {
    if let Json::Object(fields) = body {
        fields.shift_remove("_hidden_params");
    }
}

// The fields of the logged reply the cost service reads back, as ModelResponse.model_dump() writes them.
pub(super) fn completion_response(body: &Json) -> CompletionResponse {
    let Json::Object(fields) = body else {
        return CompletionResponse::default();
    };
    let model = match fields.get("model") {
        Some(Json::Text(name)) if !name.is_empty() => Some(name.clone()),
        None
        | Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        ) => None,
    };
    let usage = match fields.get("usage") {
        None | Some(Json::Null) => None,
        Some(value) => serde_json::to_vec(value)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok()),
    };
    CompletionResponse {
        id: None,
        model,
        usage,
    }
}
