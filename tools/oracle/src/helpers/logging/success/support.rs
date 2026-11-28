use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::exporter_options::failure_counter::CallbackFailureCounter;
use betterlite_logging::exporter_options::redaction::ResponseStamp;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::ports::{
    StandardPayloadBuilder, StreamAssembler, SuccessClock,
};
use betterlite_logging::logging_success::types::{SuccessMode, SuccessResult};
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use serde_json::Number;

use super::input::MessageFlagsSpec;

#[derive(Clone, Debug)]
pub(crate) struct LoggedEvent {
    pub(crate) logger: String,
    pub(crate) event: String,
    pub(crate) stream: bool,
    pub(crate) model: String,
    pub(crate) messages: Option<Json>,
    pub(crate) response: Option<Json>,
}

pub(crate) struct RecordingLogger {
    name: String,
    flags: MessageFlagsSpec,
    fail: bool,
    stamp: ResponseStamp,
    calls: Mutex<Vec<LoggedEvent>>,
}

impl RecordingLogger {
    pub(crate) fn new(
        name: &str,
        flags: &MessageFlagsSpec,
        fail: bool,
        stamp: &ResponseStamp,
    ) -> Self {
        Self {
            name: name.to_owned(),
            flags: flags.clone(),
            fail,
            stamp: stamp.clone(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<LoggedEvent> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, event_name: &str, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LoggedEvent {
                logger: self.name.clone(),
                event: event_name.to_owned(),
                stream: event.stream,
                model: event.payload.model.clone(),
                messages: event.payload.messages.clone(),
                response: event
                    .payload
                    .response
                    .as_ref()
                    .map(|response| normalized(response, &self.stamp)),
            });
        if self.fail {
            return Err(LoggerError::Delivery {
                callback: self.name.clone(),
                message: "the transport is down".to_owned(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl CallbackLogger for RecordingLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    fn turn_off_message_logging(&self) -> bool {
        self.flags.turn_off_message_logging
    }

    fn redacts_messages_itself(&self) -> bool {
        self.flags.redacts_messages_itself
    }

    fn message_logging(&self) -> bool {
        self.flags.message_logging.unwrap_or(true)
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.record("log_success_event", event)
    }

    fn log_stream_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.record("log_stream_event", event)
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record("async_log_success_event", &event)
    }

    async fn async_log_stream_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record("async_log_stream_event", &event)
    }
}

// the minted redacted id is random, so the record carries the same stamp the LiteLLM side pins
fn normalized(response: &Json, stamp: &ResponseStamp) -> Json {
    let Json::Object(fields) = response else {
        return response.clone();
    };
    let Some(Json::Text(id)) = fields.get("id") else {
        return response.clone();
    };
    let Some(rest) = id.strip_prefix("chatcmpl-") else {
        return response.clone();
    };
    let bytes = rest.as_bytes();
    let minted = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        });
    if !minted {
        return response.clone();
    }
    let mut stamped = fields.clone();
    stamped.insert("id".to_owned(), Json::Text(stamp.id.clone()));
    stamped.insert(
        "created".to_owned(),
        Json::Number(Number::from(stamp.created)),
    );
    Json::Object(stamped)
}

pub(crate) struct FixedBuilder {
    pub(crate) payload: Option<StandardLoggingPayload>,
}

impl StandardPayloadBuilder for FixedBuilder {
    fn build(
        &self,
        _state: &LoggingState,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        self.payload.clone()
    }
}

pub(crate) struct FixedAssembler {
    pub(crate) complete: Option<Json>,
    pub(crate) declines_results: bool,
}

impl StreamAssembler for FixedAssembler {
    fn assemble(
        &self,
        state: &LoggingState,
        result: &SuccessResult,
        _mode: SuccessMode,
    ) -> Option<Json> {
        if state.stream != Some(true) {
            return None;
        }
        match result {
            SuccessResult::Chunk(_) => self.complete.clone(),
            SuccessResult::Chat(_) | SuccessResult::Embeddings(_) | SuccessResult::Other(_) => {
                (!self.declines_results).then(|| result.response().clone())
            }
        }
    }
}

pub(crate) struct FixedClock {
    pub(crate) now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

#[derive(Default)]
pub(crate) struct RecordingCounter(Mutex<Vec<String>>);

impl RecordingCounter {
    pub(crate) fn calls(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CallbackFailureCounter for RecordingCounter {
    // ast-grep-ignore: no-long-names
    fn increment_callback_logging_failure(&self, callback_name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(callback_name.to_owned());
    }
}

// logging_success.py keeps both counters in litellm.callbacks, so each one is a callback that answers a counter
#[async_trait]
impl CallbackLogger for RecordingCounter {
    fn callback_name(&self) -> &str {
        "_RecordingCounter"
    }

    fn failure_counter(&self) -> Option<&dyn CallbackFailureCounter> {
        Some(self)
    }
}
