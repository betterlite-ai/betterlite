use std::str::FromStr as _;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_privacy::snapshot::redact_user_api_key_info;
use crate::otel::callbacks::{IdGenerator, object_of, text_of};

pub const CALLBACK_NAME: &str = "logfire";
pub const CHAT_COMPLETION_TEMPLATE: &str = "Chat Completion with {request_data[model]!r}";
pub const EMBEDDING_TEMPLATE: &str = "Embedding Creation with {request_data[model]!r}";
pub const IMAGE_GENERATION_TEMPLATE: &str = "Image Generation with {request_data[model]!r}";
pub const DEFAULT_TEMPLATE: &str = "Litellm Call with {request_data[model]!r}";
// LiteLLM strips these three before logging, so a large or circular value never reaches the sink
pub const EXCLUDED_METADATA_KEYS: [&str; 3] = ["endpoint", "caching_groups", "previous_models"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogfireLevel {
    Info,
    Error,
}

#[derive(Clone, Copy)]
pub struct SpanData<'payload> {
    pub request_data: &'payload Json,
}

pub struct SpanConfig<'payload> {
    pub message_template: &'static str,
    pub span_data: SpanData<'payload>,
}

pub trait LogfireSink: Send + Sync {
    fn info(&self, config: &SpanConfig<'_>);

    fn error(&self, config: &SpanConfig<'_>, exc_info: bool);
}

pub struct LogfireLogger {
    sink: Box<dyn LogfireSink>,
    ids: IdGenerator,
    redact_user_api_key_info: bool,
}

impl LogfireLogger {
    #[must_use]
    pub fn new(
        sink: Box<dyn LogfireSink>,
        ids: IdGenerator,
        redact_user_api_key_info: bool,
    ) -> Self {
        Self {
            sink,
            ids,
            redact_user_api_key_info,
        }
    }

    fn log(&self, event: &LoggerEvent, level: LogfireLevel) {
        let payload = build_payload(
            &event.payload,
            &event.litellm_metadata,
            self.ids,
            self.redact_user_api_key_info,
        );
        let config = SpanConfig {
            message_template: message_template(&event.payload.call_type),
            span_data: SpanData {
                request_data: &payload,
            },
        };
        match level {
            LogfireLevel::Info => self.sink.info(&config),
            LogfireLevel::Error => self.sink.error(&config, true),
        }
    }
}

fn message_template(call_type: &str) -> &'static str {
    match call_type {
        "completion" | "acompletion" => CHAT_COMPLETION_TEMPLATE,
        "embedding" | "aembedding" => EMBEDDING_TEMPLATE,
        "image_generation" | "aimage_generation" => IMAGE_GENERATION_TEMPLATE,
        _ => DEFAULT_TEMPLATE,
    }
}

// the payload logfire receives as request_data, field for field what LiteLLM's log_event builds
fn build_payload(
    payload: &StandardLoggingPayload,
    litellm_metadata: &IndexMap<String, Json>,
    ids: IdGenerator,
    redact: bool,
) -> Json {
    let response = payload
        .response
        .as_ref()
        .and_then(object_of)
        .cloned()
        .unwrap_or_default();
    let mut fields = header_fields(payload, ids, &response);
    let user = payload
        .model_parameters
        .get("user")
        .and_then(text_of)
        .unwrap_or_default();
    fields.insert("model".to_owned(), Json::Text(payload.model.clone()));
    fields.insert("user".to_owned(), Json::Text(user.to_owned()));
    fields.insert(
        "modelParameters".to_owned(),
        Json::Object(payload.model_parameters.clone()),
    );
    fields.insert(
        "spend".to_owned(),
        serde_json::Number::from_str(&payload.response_cost.to_string())
            .map_or(Json::Null, Json::Number),
    );
    fields.insert(
        "messages".to_owned(),
        payload.messages.clone().unwrap_or(Json::Null),
    );
    fields.extend(response_fields(litellm_metadata, &response, redact));
    Json::Object(fields)
}

fn header_fields(
    payload: &StandardLoggingPayload,
    ids: IdGenerator,
    response: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let id = match response.get("id").and_then(text_of) {
        Some(id) => id.to_owned(),
        None => ids(),
    };
    IndexMap::from([
        ("id".to_owned(), Json::Text(id)),
        (
            "call_type".to_owned(),
            Json::Text(payload.call_type.clone()),
        ),
        (
            "cache_hit".to_owned(),
            Json::Bool(payload.cache_hit.unwrap_or(false)),
        ),
        ("startTime".to_owned(), timestamp(payload.start_time)),
        ("endTime".to_owned(), timestamp(payload.end_time)),
        (
            "responseTime (seconds)".to_owned(),
            serde_json::Number::from_str(&payload.response_time.to_string())
                .map_or(Json::Null, Json::Number),
        ),
    ])
}

fn response_fields(
    litellm_metadata: &IndexMap<String, Json>,
    response: &IndexMap<String, Json>,
    redact: bool,
) -> IndexMap<String, Json> {
    let usage = match response.get("usage") {
        Some(Json::Null) | None => Json::Object(IndexMap::new()),
        Some(usage) => usage.clone(),
    };
    let mut fields = IndexMap::new();
    fields.insert("response".to_owned(), Json::Object(response.clone()));
    fields.insert("usage".to_owned(), usage);
    fields.insert(
        "metadata".to_owned(),
        clean_metadata(litellm_metadata, redact),
    );
    fields
}

// LiteLLM logs the request's litellm_params metadata, minus the three keys it strips
fn clean_metadata(litellm_metadata: &IndexMap<String, Json>, redact: bool) -> Json {
    let mut fields = litellm_metadata.clone();
    for key in EXCLUDED_METADATA_KEYS {
        fields.shift_remove(key);
    }
    redact_user_api_key_info(&Json::Object(fields), redact)
}

// LiteLLM logs the datetime objects it was given, which the sink renders as RFC 3339 text
fn timestamp(seconds: Decimal) -> Json {
    seconds
        .checked_mul(Decimal::from(1_000_000_i64))
        .and_then(|micros| micros.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_micros)
        .map_or(Json::Null, |moment| Json::Text(moment.to_rfc3339()))
}

#[async_trait]
impl CallbackLogger for LogfireLogger {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.log(event, LogfireLevel::Info);
        Ok(())
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.log(event, LogfireLevel::Error);
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log(&event, LogfireLevel::Info);
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log(&event, LogfireLevel::Error);
        Ok(())
    }
}

mod tests;
