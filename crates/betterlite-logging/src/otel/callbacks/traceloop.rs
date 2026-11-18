use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::{Span, SpanKind, Status, Tracer as _};
use opentelemetry::{Array, Context, KeyValue, StringValue, Value};
use opentelemetry_sdk::trace::{SdkTracer, Span as SdkSpan};
use rust_decimal::prelude::ToPrimitive as _;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::datadog_llm_obs::values::{as_object, as_text};
use crate::fields::payload::StandardLoggingPayload;
use crate::otel::legacy_lifecycle::context::instant_from_ns;
use crate::otel::legacy_policy::error_recording::{RecordedException, record_exception_on_span};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::vocabulary::constants::attribute_keys::gen_ai;

pub const CALLBACK_NAME: &str = "traceloop";
pub const APP_NAME: &str = "Litellm-Server";
pub const DISABLE_BATCH: bool = true;
pub const SPAN_NAME: &str = "litellm.completion";
pub const EXCEPTION_TYPE: &str = "Exception";

// the names opentelemetry.semconv.ai's SpanAttributes carries, which traceloop.py imports
const LLM_CHAT_STOP_SEQUENCES: &str = "llm.chat.stop_sequences";
const LLM_FREQUENCY_PENALTY: &str = "llm.frequency_penalty";
const LLM_PRESENCE_PENALTY: &str = "llm.presence_penalty";
const LLM_REQUEST_FUNCTIONS: &str = "llm.request.functions";
const LLM_USER: &str = "llm.user";
const LLM_PROMPTS: &str = "gen_ai.prompt";
const LLM_COMPLETIONS: &str = "gen_ai.completion";
const LLM_USAGE_TOTAL_TOKENS: &str = "llm.usage.total_tokens";
const LLM_USAGE_COMPLETION_TOKENS: &str = "gen_ai.usage.completion_tokens";
const LLM_USAGE_PROMPT_TOKENS: &str = "gen_ai.usage.prompt_tokens";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceloopConfig {
    pub app_name: &'static str,
    pub disable_batch: bool,
}

impl Default for TraceloopConfig {
    fn default() -> Self {
        Self {
            app_name: APP_NAME,
            disable_batch: DISABLE_BATCH,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventLevel {
    Default,
    Error,
}

// tracer_wrapper's tracer is the SDK one, so the callback takes the SDK's tracer directly
pub struct TraceloopLogger {
    tracer: SdkTracer,
}

impl TraceloopLogger {
    fn log_event(&self, event: &LoggerEvent, level: EventLevel, status_message: Option<&str>) {
        let payload = &event.payload;
        // LiteLLM hands int(seconds) to start_span, whose argument is nanoseconds
        let builder = self
            .tracer
            .span_builder(SPAN_NAME)
            .with_kind(SpanKind::Client)
            .with_start_time(instant_from_ns(
                payload.start_time.trunc().to_i64().unwrap_or_default(),
            ));
        let mut span = self.tracer.build_with_context(builder, &Context::current());
        if span.is_recording() {
            // Python's TypeError and AttributeError inside the try leave the span unended
            let Some(()) = write_attributes(&mut span, payload) else {
                return;
            };
        }
        if level == EventLevel::Error
            && let Some(message) = status_message
        {
            let exception = RecordedException {
                type_name: EXCEPTION_TYPE.to_owned(),
                message: message.to_owned(),
                stacktrace: String::new(),
            };
            record_exception_on_span(&mut span, Some(&exception), &MetadataMap::new());
            span.set_status(Status::error(message.to_owned()));
        }
        span.end_with_timestamp(instant_from_ns(
            payload.end_time.trunc().to_i64().unwrap_or_default(),
        ));
    }
}

fn write_attributes(span: &mut SdkSpan, payload: &StandardLoggingPayload) -> Option<()> {
    write_optional_parameters(span, payload);
    write_prompts(span, payload.messages.as_ref())?;
    let response = as_object(payload.response.as_ref()?)?;
    write_attribute(span, gen_ai::RESPONSE_MODEL, response.get("model"));
    write_usage(span, response.get("usage"));
    write_completions(span, response.get("choices"))
}

fn write_optional_parameters(span: &mut SdkSpan, payload: &StandardLoggingPayload) {
    let optional_params = &payload.model_parameters;
    write_attribute(
        span,
        gen_ai::REQUEST_MODEL,
        Some(&Json::Text(payload.model.clone())),
    );
    // the typed event keeps one model-parameter snapshot, so the guard and the value share a lookup
    for (name, attribute) in [
        ("stop", LLM_CHAT_STOP_SEQUENCES),
        ("frequency_penalty", LLM_FREQUENCY_PENALTY),
        ("presence_penalty", LLM_PRESENCE_PENALTY),
        ("top_p", gen_ai::REQUEST_TOP_P),
        ("user", LLM_USER),
        ("max_tokens", gen_ai::REQUEST_MAX_TOKENS),
        ("temperature", gen_ai::REQUEST_TEMPERATURE),
    ] {
        if optional_params.contains_key(name) {
            write_attribute(span, attribute, optional_params.get(name));
        }
    }
    if let Some(request_functions) = optional_params
        .get("tools")
        .or_else(|| optional_params.get("functions"))
    {
        write_attribute(span, LLM_REQUEST_FUNCTIONS, Some(request_functions));
    }
}

fn write_prompts(span: &mut SdkSpan, messages: Option<&Json>) -> Option<()> {
    let Some(Json::List(entries)) = messages else {
        // kwargs.get("messages") is None here and enumerate(None) raises
        return None;
    };
    for (index, message) in entries.iter().enumerate() {
        write_role_and_content(span, &format!("{LLM_PROMPTS}.{index}"), as_object(message)?);
    }
    Some(())
}

fn write_role_and_content<S>(span: &mut S, prefix: &str, fields: &IndexMap<String, Json>)
where
    S: Span,
{
    write_attribute(span, &format!("{prefix}.role"), fields.get("role"));
    write_attribute(span, &format!("{prefix}.content"), fields.get("content"));
}

fn write_usage(span: &mut SdkSpan, usage: Option<&Json>) {
    let Some(usage_fields) = usage.and_then(as_object) else {
        return;
    };
    write_attribute(
        span,
        LLM_USAGE_TOTAL_TOKENS,
        usage_fields.get("total_tokens"),
    );
    write_attribute(
        span,
        LLM_USAGE_COMPLETION_TOKENS,
        usage_fields.get("completion_tokens"),
    );
    write_attribute(
        span,
        LLM_USAGE_PROMPT_TOKENS,
        usage_fields.get("prompt_tokens"),
    );
}

fn write_completions(span: &mut SdkSpan, choices: Option<&Json>) -> Option<()> {
    let Some(Json::List(entries)) = choices else {
        // response_obj.get("choices") is None here and enumerate(None) raises
        return None;
    };
    for (index, choice) in entries.iter().enumerate() {
        let fields = as_object(choice)?;
        let prefix = format!("{LLM_COMPLETIONS}.{index}");
        write_attribute(
            span,
            &format!("{prefix}.finish_reason"),
            fields.get("finish_reason"),
        );
        let message = fields.get("message").and_then(as_object)?;
        write_role_and_content(span, &prefix, message);
    }
    Some(())
}

fn write_attribute<S>(span: &mut S, key: &str, value: Option<&Json>)
where
    S: Span,
{
    let Some(attribute) = value.and_then(attribute_value) else {
        return;
    };
    span.set_attribute(KeyValue::new(key.to_owned(), attribute));
}

// Python's set_attribute drops a dict, so only scalars and string sequences become one
fn attribute_value(value: &Json) -> Option<Value> {
    match value {
        Json::Text(text) => Some(Value::from(text.clone())),
        Json::Bool(flag) => Some(Value::Bool(*flag)),
        Json::Number(number) => number.as_i64().map_or_else(
            || number.as_f64().map(Value::F64),
            |whole| Some(Value::I64(whole)),
        ),
        Json::List(items) => string_array(items),
        Json::Null | Json::Object(_) => None,
    }
}

fn string_array(items: &[Json]) -> Option<Value> {
    let mut strings = Vec::with_capacity(items.len());
    for item in items {
        strings.push(StringValue::from(as_text(item)?.to_owned()));
    }
    Some(Value::Array(Array::String(strings)))
}

#[async_trait]
impl CallbackLogger for TraceloopLogger {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.log_event(event, EventLevel::Default, None);
        Ok(())
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let message = event
            .payload
            .error_information
            .as_ref()
            .and_then(|information| information.error_message.clone().flatten())
            .or_else(|| event.payload.error_str.clone());
        self.log_event(event, EventLevel::Error, message.as_deref());
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_event(&event, EventLevel::Default, None);
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_failure_event(&event)
    }
}

mod tests;
