use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Serialize;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::otel::legacy_policy::capture::LoggingFlags;
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

use super::lifecycle::{DEFAULT_CALLBACK_NAME, LegacyLifecycle, LifecycleCall};

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
            start_time: span_time(event.payload.start_time),
            end_time: span_time(event.payload.end_time),
            exception: None,
            flags: LoggingFlags {
                turn_off_message_logging: dynamic_turn_off(event),
                // CustomLogger's deprecated flag defaults to true, and the class never overrides it
                message_logging: true,
            },
        };
        let result = if failure {
            self.lifecycle.handle_failure(&call)
        } else {
            self.lifecycle.handle_success(&call)
        };
        result.map_err(|error| LoggerError::Delivery {
            callback: self.callback_name.clone(),
            message: error.to_string(),
        })
    }

    // LiteLLM hands its callbacks the request kwargs; the typed event carries the same fields
    fn event_kwargs(&self, event: &LoggerEvent) -> Result<MetadataMap, LoggerError> {
        Ok(MetadataMap::from([
            (
                "standard_logging_object".to_owned(),
                self.encoded(&*event.payload)?,
            ),
            ("litellm_params".to_owned(), self.event_params(event)?),
            ("optional_params".to_owned(), self.optional_params(event)?),
            (
                "standard_callback_dynamic_params".to_owned(),
                self.encoded(&*event.dynamic_params)?,
            ),
            (
                "model".to_owned(),
                OtelModelValue::String(event.payload.model.clone()),
            ),
            (
                "call_type".to_owned(),
                OtelModelValue::String(event.payload.call_type.clone()),
            ),
        ]))
    }

    fn event_params(&self, event: &LoggerEvent) -> Result<OtelModelValue, LoggerError> {
        Ok(OtelModelValue::Object(MetadataMap::from([
            (
                "metadata".to_owned(),
                self.encoded(&event.payload.metadata)?,
            ),
            (
                "litellm_metadata".to_owned(),
                self.encoded(&*event.litellm_metadata)?,
            ),
            (
                "litellm_session_id".to_owned(),
                OtelModelValue::String(event.payload.session_id.clone()),
            ),
            (
                "custom_llm_provider".to_owned(),
                self.encoded(&event.payload.custom_llm_provider)?,
            ),
        ])))
    }

    fn optional_params(&self, event: &LoggerEvent) -> Result<OtelModelValue, LoggerError> {
        let mut params = event
            .payload
            .model_parameters
            .iter()
            .map(|(key, value)| self.encoded(value).map(|encoded| (key.clone(), encoded)))
            .collect::<Result<MetadataMap, LoggerError>>()?;
        // The event's stream flag is authoritative even if the parameter snapshot carries a stale value.
        params.insert("stream".to_owned(), OtelModelValue::Bool(event.stream));
        Ok(OtelModelValue::Object(params))
    }

    fn encoded<T>(&self, value: &T) -> Result<OtelModelValue, LoggerError>
    where
        T: Serialize,
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
        _ => false,
    }
}

fn span_time(seconds: Decimal) -> SpanTime {
    match seconds.to_f64() {
        Some(value) => SpanTime::Seconds(value),
        None => SpanTime::Missing,
    }
}
