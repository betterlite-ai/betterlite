use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use thiserror::Error;

use crate::datadog::apm::{Span, Tracer};
use crate::datadog_llm_metrics::error_info::error_info;
use crate::datadog_llm_metrics::metrics::{MetricsError, span_metrics};
use crate::datadog_llm_metrics::span_kind::span_kind;
use crate::datadog_llm_metrics::token_counting::{
    ToolTokenCounter, ToolTokensError, tool_output_tokens,
};
use crate::datadog_llm_obs::messages::to_dd_messages;
use crate::datadog_llm_obs::records::DdMessage;
use crate::datadog_llm_obs::response_messages::response_messages;
use crate::datadog_llm_obs::span_records::{InputMeta, LLMObsPayload, Meta, SpanStatus};
use crate::datadog_llm_obs::tool_definitions::to_dd_tool_definitions;
use crate::datadog_llm_obs::values::Object;
use crate::datadog_llm_privacy::RecordEncodingError;
use crate::datadog_llm_privacy::cost_dimension::{cost_dimension_tags, declared_cost_tags};
use crate::datadog_llm_privacy::message_redact::redact_messages;
use crate::datadog_llm_privacy::metadata::span_metadata;
use crate::datadog_llm_privacy::router_span::router_span_fields;
use crate::exporter_options::options::CustomLoggerOptions;
use crate::fields::payload::StandardLoggingPayload;
use crate::spend_bodies::redaction::should_redact_message_logging;

use super::settings::DatadogLlmObsSettings;
use super::tags::span_tags;

const NANOS_PER_SECOND: i64 = 1_000_000_000;
const DEFAULT_SPAN_NAME: &str = "litellm_llm_call";
const UNDEFINED_PARENT: &str = "undefined";

#[derive(Debug, Error)]
pub enum SpanError<Failure>
where
    Failure: StdError + 'static,
{
    #[error(transparent)]
    Record(#[from] RecordEncodingError),
    #[error(transparent)]
    Metrics(#[from] MetricsError),
    #[error(transparent)]
    ToolTokens(#[from] ToolTokensError<Failure>),
    #[error("the span timestamps do not fit a nanosecond count")]
    TimingOverflow,
}

pub struct SpanRequest<'request> {
    pub payload: &'request StandardLoggingPayload,
    pub request_metadata: &'request Object,
    pub redact: bool,
    pub apm_id: Option<&'request str>,
}

pub struct SpanContext<'context, Counter> {
    pub settings: &'context DatadogLlmObsSettings,
    pub counter: &'context Counter,
    pub generate_id: &'context (dyn Fn() -> String + Send + Sync),
}

// _payload_logging_is_off: the instance flags first, then the per-request redaction parameters
#[must_use]
pub fn logging_is_off(
    options: &CustomLoggerOptions,
    request_metadata: &Object,
    global_turn_off: bool,
) -> bool {
    let params = Object::from([(
        "metadata".to_owned(),
        Json::Object(request_metadata.clone()),
    )]);
    options.turn_off_message_logging
        || !options.message_logging
        || should_redact_message_logging(Some(&params), Some(request_metadata), global_turn_off)
}

// _get_apm_trace_id reads only the current span, so an empty or bare tracer leaves the id off
#[must_use]
pub fn apm_trace_id(tracer: &dyn Tracer) -> Option<String> {
    tracer
        .current_span()
        .and_then(Span::trace_id)
        .map(|id| id.to_string())
}

pub async fn span_payload<Counter>(
    request: &SpanRequest<'_>,
    context: &SpanContext<'_, Counter>,
) -> Result<LLMObsPayload, SpanError<Counter::Failure>>
where
    Counter: ToolTokenCounter,
{
    let payload = request.payload;
    let caller = request.request_metadata;
    let (meta, tags, tool_tokens) = span_meta(request, context).await?;
    let (start_ns, duration) = timing(payload).ok_or(SpanError::TimingOverflow)?;
    let status = if meta.error.is_some() {
        SpanStatus::Error
    } else {
        SpanStatus::Ok
    };
    Ok(LLMObsPayload {
        parent_id: Some(
            metadata_text(caller, "parent_id")
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| UNDEFINED_PARENT.to_owned()),
        ),
        trace_id: Some(payload.trace_id.clone()),
        span_id: Some(metadata_text(caller, "span_id").unwrap_or_else(|| (context.generate_id)())),
        name: Some(metadata_text(caller, "name").unwrap_or_else(|| DEFAULT_SPAN_NAME.to_owned())),
        apm_id: request.apm_id.map(str::to_owned),
        meta: Some(meta),
        start_ns: Some(start_ns),
        duration: Some(duration),
        metrics: Some(span_metrics(payload, tool_tokens)?),
        tags: Some(tags),
        status: Some(status),
    })
}

async fn span_meta<Counter>(
    request: &SpanRequest<'_>,
    context: &SpanContext<'_, Counter>,
) -> Result<(Meta, Vec<String>, Option<f64>), SpanError<Counter::Failure>>
where
    Counter: ToolTokenCounter,
{
    let payload = request.payload;
    let input_messages = to_dd_messages(payload.messages.as_ref());
    let output_messages =
        response_messages(payload.response.as_ref(), Some(payload.call_type.as_str()));
    let tool_tokens =
        tool_output_tokens(&input_messages, payload.model.as_str(), context.counter).await?;
    let error = error_info(payload);
    let router = router_span_fields(payload.metadata.routing_decision.as_ref(), request.redact);
    let mut tags = span_tags(&context.settings.environment, payload);
    tags.extend(cost_dimension_tags(payload, &router));
    let metadata = span_metadata(payload, &router, &declared_cost_tags(&tags), request.redact)?;
    let tool_definitions = if request.redact {
        Vec::new()
    } else {
        to_dd_tool_definitions(Some(&Json::Object(payload.model_parameters.clone())))
    };
    let meta = Meta {
        kind: Some(span_kind(
            Some(payload.call_type.as_str()),
            metadata_text(request.request_metadata, "parent_id")
                .filter(|id| !id.is_empty())
                .as_deref(),
        )),
        input: Some(InputMeta {
            messages: present_messages(&input_messages, request.redact),
        }),
        output: Some(InputMeta {
            messages: present_messages(&output_messages, request.redact),
        }),
        metadata: Some(metadata),
        error,
        tool_definitions,
    };
    Ok((meta, tags, tool_tokens))
}

fn present_messages(messages: &[DdMessage], redact: bool) -> Vec<DdMessage> {
    if !redact {
        return messages.to_vec();
    }
    redact_messages(messages)
        .into_iter()
        .map(DdMessage::Message)
        .collect()
}

// Python multiplies the epoch seconds by 1e9 as a float and truncates toward zero
fn nanoseconds(seconds: Decimal) -> Option<i64> {
    seconds
        .checked_mul(Decimal::from(NANOS_PER_SECOND))?
        .to_i64()
}

fn timing(payload: &StandardLoggingPayload) -> Option<(i64, i64)> {
    let elapsed = payload.end_time.checked_sub(payload.start_time)?;
    let start = nanoseconds(payload.start_time)?;
    Some((start, nanoseconds(elapsed)?))
}

fn metadata_text(metadata: &Object, key: &str) -> Option<String> {
    match metadata.get(key) {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Number(number)) => Some(number.to_string()),
        Some(Json::Bool(flag)) => Some(flag.to_string()),
        Some(Json::Null | Json::List(_) | Json::Object(_)) | None => None,
    }
}
