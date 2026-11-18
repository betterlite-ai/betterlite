use super::lifecycle::LifecycleEvent;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::StandardLoggingMetadata;
use crate::otel::legacy_policy::capture::LoggingFlags;
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::model::metadata::wire::MetadataMap;

use super::lifecycle::{DEFAULT_CALLBACK_NAME, LegacyLifecycle, LifecycleCall};

#[derive(Serialize)]
struct EventKwargs<'event> {
    standard_logging_object: &'event StandardLoggingPayload,
    litellm_params: RequestParams<'event>,
    optional_params: IndexMap<String, Json>,
    #[serde(rename = "standard_callback_dynamic_params")]
    dynamic_params: &'event Json,
    messages: &'event Option<Json>,
    model: &'event str,
    call_type: &'event str,
}

#[derive(Serialize)]
struct RequestParams<'event> {
    metadata: &'event StandardLoggingMetadata,
    litellm_metadata: &'event IndexMap<String, Json>,
    litellm_session_id: &'event str,
    custom_llm_provider: Option<&'event str>,
}

// the legacy OpenTelemetry logger as the callback trait sees it: one event payload per lifecycle call
pub struct LegacyOtelLogger {
    lifecycle: Arc<LegacyLifecycle>,
    callback_name: String,
}

impl LegacyOtelLogger {
    #[must_use]
    pub fn new(lifecycle: Arc<LegacyLifecycle>) -> Self {
        let callback_name = lifecycle
            .callback_name
            .clone()
            .unwrap_or_else(|| DEFAULT_CALLBACK_NAME.to_owned());
        Self {
            lifecycle,
            callback_name,
        }
    }

    fn run(&self, event: &LoggerEvent, failure: bool) -> Result<(), LoggerError> {
        let kwargs = self.event_kwargs(event)?;
        let response = match &event.payload.response {
            Some(value) => Some(self.encoded(value)?),
            None => None,
        };
        let call = LifecycleCall {
            kwargs: &kwargs,
            response: response.as_ref(),
            start_time: event
                .payload
                .start_time
                .to_f64()
                .map_or(SpanTime::Missing, SpanTime::Seconds),
            end_time: event
                .payload
                .end_time
                .to_f64()
                .map_or(SpanTime::Missing, SpanTime::Seconds),
            exception: None,
            flags: LoggingFlags {
                turn_off_message_logging: dynamic_turn_off(event),
                // CustomLogger's deprecated flag defaults to true, and the class never overrides it
                message_logging: true,
            },
        };
        let lifecycle_event = if failure {
            LifecycleEvent::Failure
        } else {
            LifecycleEvent::Success
        };
        self.lifecycle
            .handle(&call, lifecycle_event)
            .map_err(|error| LoggerError::Delivery {
                callback: self.callback_name.clone(),
                message: error.to_string(),
            })
    }

    // LiteLLM hands its callbacks the request kwargs; the typed event carries the same fields
    fn event_kwargs(&self, event: &LoggerEvent) -> Result<MetadataMap, LoggerError> {
        let mut optional_params = event.payload.model_parameters.clone();
        // The event's stream flag is authoritative even if the parameter snapshot carries a stale value.
        optional_params.insert("stream".to_owned(), Json::Bool(event.stream));
        self.encoded(&EventKwargs {
            standard_logging_object: &event.payload,
            litellm_params: RequestParams {
                metadata: &event.payload.metadata,
                litellm_metadata: &event.litellm_metadata,
                litellm_session_id: &event.payload.session_id,
                custom_llm_provider: event.payload.custom_llm_provider.as_deref(),
            },
            optional_params,
            dynamic_params: &event.dynamic_params,
            messages: &event.payload.messages,
            model: &event.payload.model,
            call_type: &event.payload.call_type,
        })
    }

    fn encoded<T, V>(&self, value: &T) -> Result<V, LoggerError>
    where
        T: Serialize,
        V: DeserializeOwned,
    {
        serde_json::to_value(value)
            .and_then(serde_json::from_value)
            .map_err(|error| LoggerError::Delivery {
                callback: self.callback_name.clone(),
                message: error.to_string(),
            })
    }
}

#[async_trait]
impl CallbackLogger for LegacyOtelLogger {
    fn callback_name(&self) -> &str {
        &self.callback_name
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.run(event, false)
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.run(event, true)
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.run(&event, false)
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.run(&event, true)
    }
}

// the request-level kill switch redacts the payload before the span carries any content
fn dynamic_turn_off(event: &LoggerEvent) -> bool {
    match &*event.dynamic_params {
        Json::Object(fields) => matches!(
            fields.get("turn_off_message_logging"),
            Some(Json::Bool(true))
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}
