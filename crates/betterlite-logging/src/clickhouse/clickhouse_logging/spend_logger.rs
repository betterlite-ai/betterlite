use std::future::{Ready, ready};

use super::batch_logger::ClickHouseBatchLogger;
use super::context::LensAnalysis;
use super::convert::TRACE_INGEST_ROUTE;
use super::payload::CallbackKwargs;
use super::row::{Projection, spend_log_row_from_payload};
use super::storage::ClickHouseStorage;

// a logger-wide Lens context would mark every concurrent request's rows as Lens-internal
pub struct CallbackEvent<'event> {
    pub kwargs: &'event CallbackKwargs,
    pub redact: bool,
    pub lens: &'event LensAnalysis,
}

pub struct ClickHouseSpendLogger<Storage: ClickHouseStorage> {
    pub batch: ClickHouseBatchLogger<Storage>,
}

impl<Storage: ClickHouseStorage> ClickHouseSpendLogger<Storage> {
    pub fn log(&self, event: &CallbackEvent<'_>) {
        let Some(payload) = event.kwargs.standard_logging_object.as_ref() else {
            return;
        };
        let call_type = payload.call_type.as_deref().unwrap_or_default();
        if call_type.starts_with(TRACE_INGEST_ROUTE) {
            return;
        }
        let projection = Projection {
            payload,
            kwargs: event.kwargs,
            message_logging_disabled: event.redact,
            lens: event.lens,
        };
        let row = spend_log_row_from_payload(projection);
        self.batch.enqueue([row]);
    }

    pub fn async_log_success_event(&self, event: &CallbackEvent<'_>) -> Ready<()> {
        self.log(event);
        ready(())
    }

    pub fn async_log_failure_event(&self, event: &CallbackEvent<'_>) -> Ready<()> {
        self.log(event);
        ready(())
    }
}

mod tests;
