use rust_decimal::Decimal;
use serde::Deserialize;
use thiserror::Error;

use crate::records::{StartedJob, TranscriptionJobResponse};

const MAX_MEDIA_DURATION_SECONDS: u64 = 3600;

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum TranscribeCostError {
    #[error("Transcription cost exceeds Decimal's range")]
    Overflow,
}

#[derive(Deserialize)]
struct CostMap {
    #[serde(rename = "transcribe/StartTranscriptionJob")]
    priced: Option<CostEntry>,
}

#[derive(Deserialize)]
struct CostEntry {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    input_cost_per_second: Decimal,
}

#[must_use]
pub fn transcribe_cost_per_second(model_cost: &str) -> Option<Decimal> {
    Some(
        serde_json::from_str::<CostMap>(model_cost)
            .ok()?
            .priced?
            .input_cost_per_second,
    )
}

pub fn transcription_job_cost(
    audio_seconds: Decimal,
    cost_per_second: Decimal,
) -> Result<Decimal, TranscribeCostError> {
    audio_seconds
        .ceil()
        .checked_mul(cost_per_second)
        .ok_or(TranscribeCostError::Overflow)
}

pub fn transcribe_max_job_cost(cost_per_second: Decimal) -> Result<Decimal, TranscribeCostError> {
    Decimal::from(MAX_MEDIA_DURATION_SECONDS)
        .checked_mul(cost_per_second)
        .ok_or(TranscribeCostError::Overflow)
}

#[must_use]
pub fn started_transcription_job(response_body: &str) -> StartedJob {
    serde_json::from_str::<TranscriptionJobResponse>(response_body)
        .ok()?
        .transcription_job
}
