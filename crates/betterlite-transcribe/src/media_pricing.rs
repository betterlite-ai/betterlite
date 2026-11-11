use async_trait::async_trait;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::live::ProbeError;
use crate::policy::cost::{TranscribeCostError, transcribe_max_job_cost, transcription_job_cost};
use crate::polling::{POLLING_INTERVAL_SECONDS, PollOutcome, PollSleeper, await_transcription_job};
use crate::records::{JobLookup, StartedJob, TranscriptionJobRecord};

// LiteLLM's TRANSCRIBE_MEDIA_FETCH_ATTEMPTS
pub const MEDIA_FETCH_ATTEMPTS: usize = 3;

#[derive(Debug, Error)]
pub enum PricingError {
    #[error(transparent)]
    Cost(#[from] TranscribeCostError),
}

#[async_trait]
pub trait MediaDurationProbe: Send + Sync {
    async fn media_seconds(
        &self,
        media_uri: &str,
        job_created_at: f64,
    ) -> Result<Option<Decimal>, ProbeError>;
}

pub async fn measure_media_seconds<P, S>(
    media_uri: &str,
    job_created_at: f64,
    probe: &P,
    sleeper: &S,
    attempts: usize,
) -> Option<Decimal>
where
    P: MediaDurationProbe + ?Sized,
    S: PollSleeper + ?Sized,
{
    for attempt in 1..=attempts {
        match probe.media_seconds(media_uri, job_created_at).await {
            // an unmeasurable object is final; only a failed fetch is retried
            Ok(seconds) => return seconds,
            Err(error) => {
                tracing::warn!(
                    media_uri = %media_uri,
                    attempt,
                    error = %error,
                    "Measuring Transcribe media failed"
                );
                if attempt < attempts {
                    sleeper.sleep(POLLING_INTERVAL_SECONDS).await;
                }
            }
        }
    }
    None
}

pub struct JobPricing<'job> {
    pub job_name: &'job str,
    pub cost_per_second: Decimal,
    pub max_attempts: usize,
    pub started_job: StartedJob,
}

pub async fn price_transcription_job<L, P, S>(
    pricing: JobPricing<'_>,
    lookup: &L,
    probe: &P,
    sleeper: &S,
) -> Result<Decimal, PricingError>
where
    L: JobLookup + ?Sized,
    P: MediaDurationProbe + ?Sized,
    S: PollSleeper + ?Sized,
{
    let JobPricing {
        job_name,
        cost_per_second,
        max_attempts,
        started_job,
    } = pricing;
    let Some(outcome) = await_transcription_job(job_name, lookup, sleeper, max_attempts).await
    else {
        tracing::warn!(
            job_name = %job_name,
            "Transcribe job did not finish while polling, charging maximum"
        );
        return Ok(transcribe_max_job_cost(cost_per_second)?);
    };
    let job = match outcome {
        PollOutcome::Finished(record) => {
            if record.transcription_job_status.as_deref() == Some("FAILED") {
                return Ok(Decimal::ZERO);
            }
            Some(record)
        }
        PollOutcome::Missing => started_job,
    };
    let Some((media_uri, created_at)) = job.as_ref().and_then(media_target) else {
        return Ok(transcribe_max_job_cost(cost_per_second)?);
    };
    let Some(seconds) =
        measure_media_seconds(media_uri, created_at, probe, sleeper, MEDIA_FETCH_ATTEMPTS).await
    else {
        return Ok(transcribe_max_job_cost(cost_per_second)?);
    };
    Ok(transcription_job_cost(seconds, cost_per_second)?)
}

fn media_target(job: &TranscriptionJobRecord) -> Option<(&str, f64)> {
    match (
        job.media.as_ref()?.media_file_uri.as_deref(),
        job.creation_time,
    ) {
        (Some(media_uri), Some(created_at)) => Some((media_uri, created_at)),
        _ => None,
    }
}

mod tests;
