use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

use super::types::{SuccessMode, SuccessResult};
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::LoggingState;

// B190 builds the standard payload (and the cost B068 computes for it); the success handlers ask for it
pub trait StandardPayloadBuilder: Send + Sync {
    fn build(
        &self,
        state: &LoggingState,
        response: &Json,
        start_time: DateTime<FixedOffset>,
        end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload>;
}

// B252 turns streamed chunks into the one assembled response a stream logs when it ends
pub trait StreamAssembler: Send + Sync {
    fn assemble(
        &self,
        state: &LoggingState,
        result: &SuccessResult,
        mode: SuccessMode,
    ) -> Option<Json>;
}

// the handlers read the clock once for the default end time, so a test can pin it
pub trait SuccessClock: Send + Sync {
    fn now(&self) -> DateTime<FixedOffset>;
}
