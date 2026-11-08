use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::LoggingState;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoggingEvent {
    AsyncSuccess,
    SyncSuccess,
    AsyncFailure,
    SyncFailure,
}

impl LoggingEvent {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::AsyncSuccess => "has_logged_async_success",
            Self::SyncSuccess => "has_logged_sync_success",
            Self::AsyncFailure => "has_logged_async_failure",
            Self::SyncFailure => "has_logged_sync_failure",
        }
    }
}

#[must_use]
pub fn should_run_logging(state: &LoggingState, event: LoggingEvent) -> bool {
    if state.model_call_details.get(event.key()) == Some(&Json::Bool(true)) {
        return false;
    }
    true
}

pub fn has_run_logging(state: &mut LoggingState, event: LoggingEvent) {
    if state.stream == Some(true) {
        return;
    }
    state
        .model_call_details
        .insert(event.key().to_owned(), Json::Bool(true));
}

#[must_use]
pub const fn claim_async_success_log(state: &mut LoggingState) -> bool {
    if state.deferred.async_success_scheduled {
        return false;
    }
    state.deferred.async_success_scheduled = true;
    true
}

pub fn update_completion_start_time(
    state: &mut LoggingState,
    completion_start_time: DateTime<FixedOffset>,
) {
    state.completion_start_time = Some(completion_start_time);
    state.model_call_details.insert(
        "completion_start_time".to_owned(),
        Json::Text(completion_start_time.to_rfc3339()),
    );
}

pub fn set_response_timing_metrics(
    state: &mut LoggingState,
    timing_metrics: IndexMap<String, f64>,
) {
    state.response_timing_metrics.clear();
    state.response_timing_metrics.extend(timing_metrics);
}
