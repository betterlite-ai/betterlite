use std::future::{Ready, ready};

use super::batch_logger::ClickHouseBatchLogger;
use super::context::LensAnalysis;
use super::convert::TRACE_INGEST_ROUTE;
use super::payload::CallbackKwargs;
use super::row::{Projection, spend_log_row_from_payload};
use super::storage::ClickHouseStorage;

pub struct ClickHouseSpendLogger<Storage: ClickHouseStorage> {
    pub batch: ClickHouseBatchLogger<Storage>,
    pub lens: LensAnalysis,
}

impl<Storage: ClickHouseStorage> ClickHouseSpendLogger<Storage> {
    pub fn log(&self, kwargs: &CallbackKwargs, message_logging_disabled: bool) {
        let Some(payload) = kwargs.standard_logging_object.as_ref() else {
            return;
        };
        let call_type = payload.call_type.as_deref().unwrap_or_default();
        if call_type.starts_with(TRACE_INGEST_ROUTE) {
            return;
        }
        let projection = Projection {
            payload,
            kwargs,
            message_logging_disabled,
            lens: &self.lens,
        };
        let row = spend_log_row_from_payload(projection);
        self.batch.enqueue([row]);
    }

    pub fn async_log_success_event(
        &self,
        kwargs: &CallbackKwargs,
        message_logging_disabled: bool,
    ) -> Ready<()> {
        self.log(kwargs, message_logging_disabled);
        ready(())
    }

    pub fn async_log_failure_event(
        &self,
        kwargs: &CallbackKwargs,
        message_logging_disabled: bool,
    ) -> Ready<()> {
        self.log(kwargs, message_logging_disabled);
        ready(())
    }
}

mod tests;
