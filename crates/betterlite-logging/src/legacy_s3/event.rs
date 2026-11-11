use betterlite_config::loader::EnvironmentLookup;
use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use thiserror::Error;

use crate::fields::payload::StandardLoggingPayload;
use crate::private_json::error::PrivateJsonError;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use crate::s3_transport::naming::{object_download_filename, object_key};
use crate::s3_transport::options::{PartitionGranularity, resolve_log_prompts_only};

use super::settings::Settings;

mod tests;

const MICROS_PER_SECOND: i64 = 1_000_000;
const OBJECT_TIME_FORMAT: &str = "%H-%M-%S-%6f";

#[derive(Debug, Error)]
pub enum EventError {
    #[error("the S3 logging payload could not be serialized")]
    Serialize(#[from] PrivateJsonError),
    #[error("the S3 object timestamp {seconds} is outside the calendar range")]
    Timestamp { seconds: Decimal },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectUpload {
    pub object_key: String,
    pub download_filename: String,
    pub body: String,
}

pub fn build(
    payload: &StandardLoggingPayload,
    settings: &Settings,
    environment: &dyn EnvironmentLookup,
) -> Result<ObjectUpload, EventError> {
    let start_time = naive_utc(payload.start_time)?;
    let prefix = team_prefix(settings, payload);
    let file_name = format!(
        "time-{}_{}",
        start_time.format(OBJECT_TIME_FORMAT),
        payload.id
    );
    let body = body(payload, settings, environment)?;
    Ok(ObjectUpload {
        object_key: object_key(
            &settings.path,
            &prefix,
            start_time,
            &file_name,
            PartitionGranularity::Day,
        ),
        download_filename: object_download_filename(start_time, &payload.id),
        body,
    })
}

fn team_prefix(settings: &Settings, payload: &StandardLoggingPayload) -> String {
    if !settings.preview_features || !settings.use_team_prefix {
        return String::new();
    }
    payload
        .metadata
        .user_api_key_team_alias
        .as_deref()
        .map_or_else(String::new, |alias| format!("{alias}/"))
}

fn body(
    payload: &StandardLoggingPayload,
    settings: &Settings,
    environment: &dyn EnvironmentLookup,
) -> Result<String, EventError> {
    let mut projected = payload.clone();
    if resolve_log_prompts_only(settings.log_prompts_only.as_ref(), environment) {
        projected.response = None;
    }
    Ok(safe_dumps(&projected, DEFAULT_MAX_RECURSE_DEPTH, None)?)
}

fn naive_utc(seconds: Decimal) -> Result<NaiveDateTime, EventError> {
    seconds
        .checked_mul(Decimal::from(MICROS_PER_SECOND))
        .and_then(|value| value.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_micros)
        .map(|at| at.naive_utc())
        .ok_or(EventError::Timestamp { seconds })
}
