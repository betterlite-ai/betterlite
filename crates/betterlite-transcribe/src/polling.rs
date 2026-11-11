use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde::de::IgnoredAny;
use tokio::time::sleep;

use crate::records::{
    GetTranscriptionJobRequest, JobLookup, JobLookupError, TranscriptionJobRecord,
};

// LiteLLM's TRANSCRIBE_JOB_MAX_POLLING_ATTEMPTS, two hours of ticks
pub const MAX_POLLING_ATTEMPTS: usize = 720;
// LiteLLM's TRANSCRIBE_JOB_POLLING_INTERVAL_SECONDS
pub const POLLING_INTERVAL_SECONDS: f64 = 10.0;

const MISSING_JOB_ERRORS: [&str; 2] = ["BadRequestException", "NotFoundException"];
const TERMINAL_STATUSES: [&str; 2] = ["COMPLETED", "FAILED"];

#[derive(Clone, Debug, PartialEq)]
pub enum PollOutcome {
    Finished(TranscriptionJobRecord),
    Missing,
}

#[async_trait]
pub trait PollSleeper: Send + Sync {
    async fn sleep(&self, seconds: f64);
}

pub struct TokioSleeper;

#[async_trait]
impl PollSleeper for TokioSleeper {
    async fn sleep(&self, seconds: f64) {
        // asyncio.sleep returns at once for a negative or NaN delay
        if let Ok(period) = Duration::try_from_secs_f64(seconds) {
            sleep(period).await;
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ErrorMember {
    Text(String),
    Other(IgnoredAny),
}

#[must_use]
pub fn aws_error_type(body: &[u8]) -> Option<String> {
    // a map type, since a derived struct would also read a JSON array
    let mut object = serde_json::from_slice::<HashMap<String, ErrorMember>>(body).ok()?;
    let ErrorMember::Text(member) = object.remove("__type")? else {
        return None;
    };
    let stripped = member
        .rsplit_once('#')
        .map_or(member.as_str(), |(_prefix, suffix)| suffix);
    Some(stripped.to_owned())
}

async fn poll_once<L>(job_name: &str, lookup: &L) -> Option<PollOutcome>
where
    L: JobLookup + ?Sized,
{
    let request = GetTranscriptionJobRequest {
        transcription_job_name: job_name.to_owned(),
    };
    match lookup.get_transcription_job(&request).await {
        Ok(response) => response
            .transcription_job
            .filter(|job| {
                job.transcription_job_status
                    .as_deref()
                    .is_some_and(|status| TERMINAL_STATUSES.contains(&status))
            })
            .map(PollOutcome::Finished),
        Err(JobLookupError::Rejected {
            aws_error_type: Some(error_type),
            ..
        }) if MISSING_JOB_ERRORS.contains(&error_type.as_str()) => {
            tracing::warn!(
                job_name = %job_name,
                error_type = %error_type,
                "Transcribe job no longer exists, pricing the media it was started with"
            );
            Some(PollOutcome::Missing)
        }
        Err(error) => {
            tracing::warn!(job_name = %job_name, error = %error, "Polling Transcribe job failed, retrying");
            None
        }
    }
}

pub async fn await_transcription_job<L, S>(
    job_name: &str,
    lookup: &L,
    sleeper: &S,
    max_attempts: usize,
) -> Option<PollOutcome>
where
    L: JobLookup + ?Sized,
    S: PollSleeper + ?Sized,
{
    for _attempt in 0..max_attempts {
        if let Some(outcome) = poll_once(job_name, lookup).await {
            return Some(outcome);
        }
        sleeper.sleep(POLLING_INTERVAL_SECONDS).await;
    }
    None
}

mod tests;
