mod list;
mod remix;

use betterlite_core::api::video_remix_call::config::DEFAULT_TIMEOUT_SECONDS;
use betterlite_core::api::video_remix_call::error::VideoCallFailure;
use betterlite_core::api::video_remix_list_transport::records::Params;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use super::harness::Bench;
use super::project;
use super::{Input, Operation, failed};

pub(crate) struct Outcome {
    pub(crate) response: Value,
    pub(crate) error: Value,
}

pub(crate) fn run(input: &Input, bench: &mut Bench) -> Result<Outcome, Box<HelperError>> {
    match input.operation {
        Operation::Remix => remix::run(input, bench, false),
        Operation::AvideoRemix => remix::run(input, bench, true),
        Operation::List => list::run(input, bench, false),
        Operation::AvideoList => list::run(input, bench, true),
    }
}

// What one entry answered: its JSON value, or the class of the failure it reported
pub(super) fn outcome<T: Serialize>(result: Result<T, VideoCallFailure>) -> Outcome {
    match result {
        Ok(value) => Outcome {
            response: serde_json::to_value(value).unwrap_or(Value::Null),
            error: Value::Null,
        },
        Err(failure) => Outcome {
            response: Value::Null,
            error: project::error(&failure.class),
        },
    }
}

pub(super) fn current_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failed(&error))
}

// The signature's `timeout: Optional[float] = 600`, then the caller's explicit None or number
pub(super) fn call_timeout(input: &Input) -> Option<f64> {
    match input.timeout.as_ref() {
        None => Some(DEFAULT_TIMEOUT_SECONDS),
        Some(Value::Null) => None,
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_)) => None,
    }
}

pub(super) fn params(value: Option<&Value>) -> Result<Option<Params>, Box<HelperError>> {
    value
        .map(|value| {
            serde_json::from_value::<Params>(value.clone()).map_err(|error| failed(&error))
        })
        .transpose()
}

pub(super) fn mock(value: Option<&Value>) -> Result<Option<Json>, Box<HelperError>> {
    value
        .map(|value| serde_json::from_value::<Json>(value.clone()).map_err(|error| failed(&error)))
        .transpose()
}

pub(super) fn extra_headers(input: &Input) -> Option<IndexMap<String, String>> {
    if input.extra_headers.is_empty() {
        return None;
    }
    Some(
        input
            .extra_headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}
