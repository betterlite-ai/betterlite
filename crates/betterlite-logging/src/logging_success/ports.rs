use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

use super::types::{SuccessMode, SuccessResult};
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::LoggingState;

pub trait StandardPayloadBuilder: Send + Sync {
    fn build(
        &self,
        state: &LoggingState,
        response: &Json,
        start_time: DateTime<FixedOffset>,
        end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload>;
}

pub trait StreamAssembler: Send + Sync {
    fn assemble(
        &self,
        state: &LoggingState,
        result: &SuccessResult,
        mode: SuccessMode,
    ) -> Option<Json>;
}

pub trait SuccessClock: Send + Sync {
    fn now(&self) -> DateTime<FixedOffset>;
}
