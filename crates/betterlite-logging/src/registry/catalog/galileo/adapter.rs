use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Offset as _, Utc};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::payload::response_object;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::galileo::admission::{self, CallbackKwargs, StandardLoggingObject, SuccessEvent};
use crate::galileo::delivery::GalileoDelivery;
use crate::galileo::delivery::transport::{TraceIds, Transport};
use crate::galileo_payloads::content::{GalileoKwargs, ResponseObj};

pub(super) struct GalileoAdapter {
    pub(super) delivery: Arc<GalileoDelivery>,
    pub(super) transport: Arc<dyn Transport + Send + Sync>,
    pub(super) ids: Arc<dyn TraceIds + Send + Sync>,
    pub(super) tracker: TaskTracker,
    pub(super) shutdown: CancellationToken,
}

#[derive(Clone, Copy)]
enum GalileoCall {
    Success,
    Failure,
}

fn delivery_error(message: &str) -> LoggerError {
    LoggerError::Delivery {
        callback: "galileo".to_owned(),
        message: message.to_owned(),
    }
}

// GalileoObserve.async_log_success_event and async_log_failure_event are its only hooks; the synchronous callback base class has no Galileo methods.
#[async_trait]
impl CallbackLogger for GalileoAdapter {
    fn callback_name(&self) -> &'static str {
        "galileo"
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.dispatch(&event, GalileoCall::Success).await
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.dispatch(&event, GalileoCall::Failure).await
    }
}

impl GalileoAdapter {
    async fn dispatch(&self, event: &LoggerEvent, call: GalileoCall) -> Result<(), LoggerError> {
        if self.shutdown.is_cancelled() {
            return Ok(());
        }
        let payload = &event.payload;
        let kwargs = CallbackKwargs {
            standard_logging_object: Some(StandardLoggingObject {
                call_type: Some(payload.call_type.clone()),
                model: Some(payload.model.clone()),
                prompt_tokens: Some(payload.prompt_tokens),
                completion_tokens: Some(payload.completion_tokens),
                total_tokens: Some(payload.total_tokens),
                response_cost: Some(payload.response_cost),
                start_time: Some(payload.start_time),
                end_time: Some(payload.end_time),
            }),
            model: Some(payload.model.clone()),
            content: GalileoKwargs {
                call_type: Some(payload.call_type.clone()),
                messages: payload.messages.clone(),
                functions: payload.model_parameters.get("functions").cloned(),
                tools: payload.model_parameters.get("tools").cloned(),
                input: None,
            },
        };
        match call {
            GalileoCall::Failure => {
                admission::async_log_failure_event(&kwargs);
                Ok(())
            }
            GalileoCall::Success => {
                let response = payload
                    .response
                    .as_ref()
                    .map(|response| response_object(&payload.call_type, response));
                self.admit(kwargs, response).await
            }
        }
    }

    // The production transport drives the pooled async client, so the synchronous admission and flush run on a blocking thread.
    async fn admit(
        &self,
        kwargs: CallbackKwargs,
        response: Option<ResponseObj>,
    ) -> Result<(), LoggerError> {
        let delivery = Arc::clone(&self.delivery);
        let transport = Arc::clone(&self.transport);
        let ids = Arc::clone(&self.ids);
        let task = self.tracker.spawn_blocking(move || {
            let event = SuccessEvent {
                kwargs: &kwargs,
                response: response.as_ref(),
                start_time: None,
                end_time: None,
                local_offset: Utc.fix(),
            };
            admission::async_log_success_event(&delivery, transport.as_ref(), ids.as_ref(), &event);
        });
        task.await
            .map_err(|error| delivery_error(&error.to_string()))
    }
}
