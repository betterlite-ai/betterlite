use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;

use super::payload::{callback_kwargs, response_fields, utc_at};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::delivery::Delivery;
use crate::opik::delivery::callback::Call;
use crate::opik::delivery::logger::OpikLogger;

pub(super) struct OpikAdapter<D> {
    pub(super) logger: Arc<OpikLogger<D>>,
    pub(super) tracker: TaskTracker,
}

struct Prepared {
    kwargs: IndexMap<String, Json>,
    response: IndexMap<String, Json>,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
}

impl Prepared {
    const fn call(&self) -> Call<'_> {
        Call {
            kwargs: &self.kwargs,
            response: &self.response,
            start_time: self.start_time,
            end_time: self.end_time,
        }
    }
}

fn invalid_time(field: &str) -> LoggerError {
    LoggerError::Delivery {
        callback: "opik".to_owned(),
        message: format!("the standard logging payload {field} is not a valid timestamp"),
    }
}

fn prepare(event: &LoggerEvent) -> Result<Prepared, LoggerError> {
    let payload = &event.payload;
    Ok(Prepared {
        kwargs: callback_kwargs(payload),
        response: response_fields(payload),
        start_time: utc_at(payload.start_time).ok_or_else(|| invalid_time("startTime"))?,
        end_time: utc_at(payload.end_time).ok_or_else(|| invalid_time("endTime"))?,
    })
}

// LiteLLM has no failure event for Opik, so only the two success hooks are forwarded
#[async_trait]
impl<D: Delivery + 'static> CallbackLogger for OpikAdapter<D> {
    fn callback_name(&self) -> &'static str {
        "opik"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let prepared = prepare(event)?;
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            logger.log_success_event(prepared.call()).await;
        });
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let prepared = prepare(&event)?;
        self.logger.async_log_success_event(prepared.call()).await;
        Ok(())
    }
}
