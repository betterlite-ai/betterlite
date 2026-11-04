use rust_decimal::Decimal;
use serde_json::Value;
use thiserror::Error;

use crate::records::{StartedJob, TranscriptionJobResponse};

const PRICED_MODEL: &str = "transcribe/StartTranscriptionJob";
const MAX_MEDIA_DURATION_SECONDS: u64 = 3600;

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum TranscribeCostError {
    #[error("Transcription cost exceeds Decimal's range")]
    Overflow,
}

#[must_use]
pub fn transcribe_cost_per_second(model_cost: &Value) -> Option<Decimal> {
    let rate = model_cost.get(PRICED_MODEL)?.get("input_cost_per_second")?;
    match rate {
        Value::Number(number) => Decimal::from_str_exact(&number.to_string()).ok(),
        Value::String(value) => Decimal::from_str_exact(value).ok(),
        _ => None,
    }
}

#[must_use]
pub fn transcription_job_cost(
    audio_seconds: Decimal,
    cost_per_second: Decimal,
) -> Result<Decimal, TranscribeCostError> {
    match audio_seconds.ceil().checked_mul(cost_per_second) {
        Some(cost) => Ok(cost),
        None => Err(TranscribeCostError::Overflow),
    }
}

#[must_use]
pub fn transcribe_max_job_cost(cost_per_second: Decimal) -> Result<Decimal, TranscribeCostError> {
    match Decimal::from(MAX_MEDIA_DURATION_SECONDS).checked_mul(cost_per_second) {
        Some(cost) => Ok(cost),
        None => Err(TranscribeCostError::Overflow),
    }
}

#[must_use]
pub fn started_transcription_job(response_body: &Value) -> StartedJob {
    let response: Result<TranscriptionJobResponse, _> =
        serde_json::from_value(response_body.clone());
    response
        .ok()
        .and_then(|response| response.transcription_job)
}
