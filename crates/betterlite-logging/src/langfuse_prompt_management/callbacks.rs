use std::fmt::Display;
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
use crate::dynamic_logger_cache::{DynamicLoggerCache, RemovalEffect};
use crate::langfuse_handler::LangFuseHandler;
use crate::langfuse_logger::langfuse::{LangFuseLogger, LangfuseCall, LangfuseLoggedEvent};
use crate::langfuse_observations::identity::Timestamp;
use crate::langfuse_projections::content::ResponseObj;

use super::bridge::run_async_function;
use super::{
    INTEGRATION_NAME, LangfusePromptManagement, LangfusePromptManagementError, unix_seconds,
};

// the kwargs LangfusePromptManagement's event body derives before log_event_on_langfuse
struct EventParts {
    litellm_params: Json,
    status_message: Option<String>,
    response: Option<Json>,
    level: &'static str,
    completion_start_time: Option<Timestamp>,
}

#[derive(Clone, Copy)]
enum EventOutcome<'outcome> {
    Success,
    Failure { status_message: &'outcome str },
}

// async_log_failure_event falls back to "Unknown error" when the payload carries no error string
fn event_parts(event: &LoggerEvent, outcome: EventOutcome<'_>) -> EventParts {
    let (level, status_message) = match outcome {
        EventOutcome::Success => ("DEFAULT", None),
        EventOutcome::Failure { status_message } => ("ERROR", Some(status_message.to_owned())),
    };
    let metadata = Json::Object((*event.litellm_metadata).clone());
    let response = match outcome {
        EventOutcome::Success => event.payload.response.clone(),
        EventOutcome::Failure { .. } => None,
    };
    let started = event.payload.completion_start_time;
    let completion_start_time = if started.is_zero() {
        None
    } else {
        Some(timestamp_at(started))
    };
    EventParts {
        litellm_params: Json::Object(IndexMap::from([("metadata".to_owned(), metadata)])),
        status_message,
        response,
        level,
        completion_start_time,
    }
}

// LiteLLM hands _serialize_datetime a datetime, so epoch seconds become the RFC 3339 text it writes
fn timestamp_at(seconds: Decimal) -> Timestamp {
    seconds
        .checked_mul(Decimal::from(1_000_000_i64))
        .and_then(|micros| micros.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_micros)
        .map_or(Timestamp::Seconds(seconds), Timestamp::DateTime)
}

// every callback failure becomes one delivery error that names langfuse and keeps the cause's text
fn delivery_error<E>(source: E) -> LoggerError
where
    E: Display,
{
    LoggerError::Delivery {
        callback: INTEGRATION_NAME.to_owned(),
        message: source.to_string(),
    }
}

// langfuse_sdk's response kind for the call, which decides how the output is projected
fn response_object<'response>(
    call_type: &str,
    response: &'response Json,
) -> Option<ResponseObj<'response>> {
    match call_type {
        "completion" | "acompletion" => Some(ResponseObj::Chat(response)),
        "embedding" | "aembedding" => Some(ResponseObj::Embedding),
        "text_completion" | "atext_completion" => Some(ResponseObj::TextCompletion(response)),
        "speech" | "aspeech" => Some(ResponseObj::Speech),
        "image_generation" | "aimage_generation" | "image_edit" | "aimage_edit" => {
            Some(ResponseObj::Image(response))
        }
        "transcription" | "atranscription" => Some(ResponseObj::Transcription(response)),
        "rerank" | "arerank" => Some(ResponseObj::Rerank(response)),
        "responses" | "aresponses" => Some(ResponseObj::ResponsesApi(response)),
        "_arealtime" => matches!(response, Json::List(_)).then_some(ResponseObj::List(response)),
        "pass_through_endpoint" => {
            matches!(response, Json::Object(_)).then_some(ResponseObj::Dict(response))
        }
        _ => None,
    }
}

#[async_trait]
impl CallbackLogger for LangfusePromptManagement {
    fn callback_name(&self) -> &str {
        INTEGRATION_NAME
    }

    // CustomLogger.log_success_event: asyncify.run_async_function drives the async hook to completion
    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let event = Arc::new(event.clone());
        run_async_function(self.async_log_success_event(event)).map_err(delivery_error)?
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let event = Arc::new(event.clone());
        run_async_function(self.async_log_failure_event(event)).map_err(delivery_error)?
    }

    // exceptions are logged and counted, never propagated to the request
    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let parts = event_parts(&event, EventOutcome::Success);
        self.deliver_logged_event(&event, parts).await
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let status_message = event
            .payload
            .error_str
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("Unknown error");
        let parts = event_parts(&event, EventOutcome::Failure { status_message });
        self.deliver_logged_event(&event, parts).await
    }
}

impl LangfusePromptManagement {
    // LangFuseHandler.get_langfuse_logger_for_request with this instance as the global logger
    async fn selected_logger(
        &self,
        event: &LoggerEvent,
    ) -> Result<Arc<LangFuseLogger>, LoggerError> {
        // the cache is built per call: this class never keeps dynamic loggers between requests
        let removal: RemovalEffect<Arc<LangFuseLogger>> = Arc::new(|logger| logger.stop());
        let cache = DynamicLoggerCache::new(removal, Arc::new(unix_seconds));
        LangFuseHandler::logger_for_request(
            &self.services,
            &cache,
            event.dynamic_params.as_ref(),
            Some(Arc::clone(&self.logger)),
        )
        .await
        .map_err(delivery_error)
    }

    // the try block of async_log_success_event and async_log_failure_event
    async fn deliver_logged_event(
        &self,
        event: &LoggerEvent,
        parts: EventParts,
    ) -> Result<(), LoggerError> {
        let logger = self.selected_logger(event).await?;
        let object = parts
            .response
            .as_ref()
            .and_then(|response| response_object(&event.payload.call_type, response));
        let call = LangfuseCall {
            payload: event.payload.as_ref(),
            litellm_params: Some(&parts.litellm_params),
            messages: event.payload.messages.as_ref(),
            optional_params: &event.payload.model_parameters,
            response: parts.response.as_ref(),
            response_object: object,
            start_time: Some(timestamp_at(event.payload.start_time)),
            end_time: Some(timestamp_at(event.payload.end_time)),
            completion_start_time: parts.completion_start_time,
            level: parts.level,
            status_message: parts.status_message.as_deref(),
            litellm_trace_id: Some(event.payload.trace_id.as_str()),
            vertex_location: None,
            aws_region_name: None,
            preset_cache_key: None,
            input: None,
            masker: None,
        };
        let logged = logger.log_event_on_langfuse(&call).await;
        let remembered = self.remember_trace_id(event, &logged);
        // LiteLLM stops a dynamic-cache logger on expiry; this cache lasts one call, so stop it here
        if !Arc::ptr_eq(&logger, &self.logger) {
            logger.stop();
        }
        remembered.map_err(delivery_error)
    }

    // _remember_trace_id stores the trace only when the call id is a string and a trace id came back
    fn remember_trace_id(
        &self,
        event: &LoggerEvent,
        logged: &LangfuseLoggedEvent,
    ) -> Result<(), LangfusePromptManagementError> {
        let Some(call_id) = event.payload.litellm_call_id.as_deref() else {
            return Ok(());
        };
        let Some(trace_id) = logged.trace_id.as_deref() else {
            return Ok(());
        };
        self.trace_cache
            .set_cache(call_id, INTEGRATION_NAME, trace_id)?;
        Ok(())
    }
}
