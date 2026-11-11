#![cfg(test)]

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::live::ProbeError;
use crate::media_pricing::{
    JobPricing, MediaDurationProbe, measure_media_seconds, price_transcription_job,
};
use crate::polling::PollSleeper;
use crate::records::{
    GetTranscriptionJobRequest, JobLookup, JobLookupError, JobLookupFuture, StartedJob,
    TranscriptionJobRecord, TranscriptionMedia,
};

const COMPLETED_WITH_MEDIA: &str = r#"{"TranscriptionJob":{"TranscriptionJobStatus":"COMPLETED","CreationTime":100.0,"Media":{"MediaFileUri":"s3://audio/a.wav"}}}"#;
const COMPLETED_WITHOUT_MEDIA: &str =
    r#"{"TranscriptionJob":{"TranscriptionJobStatus":"COMPLETED","CreationTime":100.0}}"#;
const COMPLETED_WITHOUT_CREATION: &str = r#"{"TranscriptionJob":{"TranscriptionJobStatus":"COMPLETED","Media":{"MediaFileUri":"s3://audio/a.wav"}}}"#;
const FAILED_WITH_MEDIA: &str = r#"{"TranscriptionJob":{"TranscriptionJobStatus":"FAILED","CreationTime":100.0,"Media":{"MediaFileUri":"s3://audio/a.wav"}}}"#;
const RUNNING: &str = r#"{"TranscriptionJob":{"TranscriptionJobStatus":"RUNNING"}}"#;

fn rate() -> Decimal {
    Decimal::new(2, 3)
}

fn max_cost() -> Decimal {
    Decimal::new(576, 1)
}

fn record(media_uri: Option<&str>, creation_time: Option<f64>) -> TranscriptionJobRecord {
    TranscriptionJobRecord {
        transcription_job_status: Some("COMPLETED".to_owned()),
        creation_time,
        media: media_uri.map(|uri| TranscriptionMedia {
            media_file_uri: Some(uri.to_owned()),
        }),
        tags: Vec::new(),
    }
}

enum LookupReply {
    Body(&'static str),
    Missing,
    Unavailable,
}

struct FixedLookup {
    reply: LookupReply,
    calls: Mutex<usize>,
}

impl FixedLookup {
    fn new(reply: LookupReply) -> Self {
        Self {
            reply,
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl JobLookup for FixedLookup {
    fn get_transcription_job<'lookup>(
        &'lookup self,
        _request: &'lookup GetTranscriptionJobRequest,
    ) -> JobLookupFuture<'lookup> {
        Box::pin(async move {
            let mut calls = self.calls.lock().unwrap();
            *calls = calls.saturating_add(1);
            drop(calls);
            match self.reply {
                LookupReply::Body(body) => {
                    serde_json::from_str(body).map_err(JobLookupError::InvalidResponse)
                }
                LookupReply::Missing => Err(JobLookupError::Rejected {
                    status: 400,
                    aws_error_type: Some("NotFoundException".to_owned()),
                }),
                LookupReply::Unavailable => Err(JobLookupError::Unavailable),
            }
        })
    }
}

struct ScriptedProbe {
    replies: Mutex<VecDeque<Result<Option<Decimal>, ProbeError>>>,
    calls: Mutex<usize>,
    seen: Mutex<Vec<(String, f64)>>,
}

impl ScriptedProbe {
    fn new(replies: Vec<Result<Option<Decimal>, ProbeError>>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(0),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }

    fn seen(&self) -> Vec<(String, f64)> {
        self.seen.lock().unwrap().clone()
    }
}

fn probe_failure() -> ProbeError {
    ProbeError::Status {
        url: Box::from("https://audio.s3.us-east-1.amazonaws.com/a.wav"),
        status: 503,
    }
}

#[async_trait]
impl MediaDurationProbe for ScriptedProbe {
    async fn media_seconds(
        &self,
        media_uri: &str,
        job_created_at: f64,
    ) -> Result<Option<Decimal>, ProbeError> {
        self.seen
            .lock()
            .unwrap()
            .push((media_uri.to_owned(), job_created_at));
        let mut calls = self.calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        drop(calls);
        self.replies.lock().unwrap().pop_front().unwrap_or(Ok(None))
    }
}

#[derive(Default)]
struct RecordingSleeper {
    slept: Mutex<Vec<f64>>,
}

#[async_trait]
impl PollSleeper for RecordingSleeper {
    async fn sleep(&self, seconds: f64) {
        self.slept.lock().unwrap().push(seconds);
    }
}

impl RecordingSleeper {
    fn slept(&self) -> Vec<f64> {
        self.slept.lock().unwrap().clone()
    }
}

fn pricing(job_name: &str, attempts: usize, started: StartedJob) -> JobPricing<'_> {
    JobPricing {
        job_name,
        cost_per_second: rate(),
        max_attempts: attempts,
        started_job: started,
    }
}

#[tokio::test]
async fn measure_returns_an_unmeasurable_result_without_a_retry() {
    let probe = ScriptedProbe::new(vec![Ok(None)]);
    let sleeper = RecordingSleeper::default();
    let seconds = measure_media_seconds("s3://audio/a.wav", 100.0, &probe, &sleeper, 3).await;
    assert_eq!(seconds, None);
    assert_eq!(probe.calls(), 1);
    assert_eq!(sleeper.slept(), Vec::<f64>::new());
}

#[tokio::test]
async fn measure_returns_a_measurement_without_sleeping() {
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::new(15, 1)))]);
    let sleeper = RecordingSleeper::default();
    let seconds = measure_media_seconds("s3://audio/a.wav", 100.0, &probe, &sleeper, 3).await;
    assert_eq!(seconds, Some(Decimal::new(15, 1)));
    assert_eq!(probe.calls(), 1);
    assert_eq!(sleeper.slept(), Vec::<f64>::new());
}

#[tokio::test]
async fn measure_retries_a_probe_error_and_sleeps_only_between_tries() {
    let probe = ScriptedProbe::new(vec![Err(probe_failure()), Err(probe_failure())]);
    let sleeper = RecordingSleeper::default();
    let seconds = measure_media_seconds("s3://audio/a.wav", 100.0, &probe, &sleeper, 3).await;
    assert_eq!(seconds, None);
    assert_eq!(probe.calls(), 3);
    assert_eq!(sleeper.slept(), vec![10.0_f64, 10.0_f64]);
}

#[tokio::test]
async fn measure_returns_a_measurement_that_follows_a_failure() {
    let probe = ScriptedProbe::new(vec![Err(probe_failure()), Ok(Some(Decimal::ONE))]);
    let sleeper = RecordingSleeper::default();
    let seconds = measure_media_seconds("s3://audio/a.wav", 100.0, &probe, &sleeper, 3).await;
    assert_eq!(seconds, Some(Decimal::ONE));
    assert_eq!(probe.calls(), 2);
    assert_eq!(sleeper.slept(), vec![10.0_f64]);
}

#[tokio::test]
async fn measure_makes_no_call_when_attempts_are_zero() {
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::ONE))]);
    let sleeper = RecordingSleeper::default();
    let seconds = measure_media_seconds("s3://audio/a.wav", 100.0, &probe, &sleeper, 0).await;
    assert_eq!(seconds, None);
    assert_eq!(probe.calls(), 0);
}

#[tokio::test]
async fn price_charges_the_maximum_when_the_job_never_finishes() {
    let lookup = FixedLookup::new(LookupReply::Body(RUNNING));
    let probe = ScriptedProbe::new(Vec::new());
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 1, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, max_cost());
    assert_eq!(probe.calls(), 0);
    assert_eq!(sleeper.slept(), vec![10.0_f64]);
}

#[tokio::test]
async fn price_charges_zero_for_a_failed_job() {
    let lookup = FixedLookup::new(LookupReply::Body(FAILED_WITH_MEDIA));
    let probe = ScriptedProbe::new(Vec::new());
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 1, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, Decimal::ZERO);
    assert_eq!(probe.calls(), 0);
}

#[tokio::test]
async fn price_measures_the_polled_jobs_media() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITH_MEDIA));
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::new(15, 1)))]);
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 1, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, Decimal::new(4, 3));
    assert_eq!(
        probe.seen(),
        vec![("s3://audio/a.wav".to_owned(), 100.0_f64)]
    );
    assert_eq!(sleeper.slept(), Vec::<f64>::new());
}

#[tokio::test]
async fn price_measures_the_started_job_when_the_job_is_gone() {
    let lookup = FixedLookup::new(LookupReply::Missing);
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::ONE))]);
    let sleeper = RecordingSleeper::default();
    let started = Some(record(Some("s3://audio/started.wav"), Some(42.0_f64)));
    let cost = price_transcription_job(pricing("job-1", 1, started), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, rate());
    assert_eq!(
        probe.seen(),
        vec![("s3://audio/started.wav".to_owned(), 42.0_f64)]
    );
}

#[tokio::test]
async fn price_charges_the_maximum_for_a_deleted_job_without_a_started_job() {
    let lookup = FixedLookup::new(LookupReply::Missing);
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::ONE))]);
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 1, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, max_cost());
    assert_eq!(probe.calls(), 0);
}

#[tokio::test]
async fn price_charges_the_maximum_when_the_completed_job_has_no_media() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITHOUT_MEDIA));
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::ONE))]);
    let sleeper = RecordingSleeper::default();
    // a completed job is priced from its own record, never from the started job
    let started = Some(record(Some("s3://audio/started.wav"), Some(42.0_f64)));
    let cost = price_transcription_job(pricing("job-1", 1, started), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, max_cost());
    assert_eq!(probe.calls(), 0);
}

#[tokio::test]
async fn price_charges_the_maximum_without_a_creation_time() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITHOUT_CREATION));
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::ONE))]);
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 1, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, max_cost());
    assert_eq!(probe.calls(), 0);
}

#[tokio::test]
async fn price_charges_the_maximum_when_the_media_is_unmeasurable() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITH_MEDIA));
    let probe = ScriptedProbe::new(vec![Ok(None)]);
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 1, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, max_cost());
    assert_eq!(probe.calls(), 1);
    assert_eq!(sleeper.slept(), Vec::<f64>::new());
}

#[tokio::test]
async fn price_charges_the_maximum_after_the_measure_fails_three_times() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITH_MEDIA));
    let probe = ScriptedProbe::new(vec![
        Err(probe_failure()),
        Err(probe_failure()),
        Err(probe_failure()),
    ]);
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 1, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, max_cost());
    assert_eq!(probe.calls(), 3);
    assert_eq!(sleeper.slept(), vec![10.0_f64, 10.0_f64]);
}

#[tokio::test]
async fn price_measures_after_a_transient_probe_failure() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITH_MEDIA));
    let probe = ScriptedProbe::new(vec![Err(probe_failure()), Ok(Some(Decimal::ONE))]);
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 1, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, rate());
    assert_eq!(probe.calls(), 2);
}

#[tokio::test]
async fn price_rounds_measured_seconds_up() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITH_MEDIA));
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::new(1001, 3)))]);
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 1, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, Decimal::new(4, 3));
}

#[tokio::test]
async fn price_retries_a_transient_lookup_failure() {
    let lookup = FixedLookup::new(LookupReply::Unavailable);
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::ONE))]);
    let sleeper = RecordingSleeper::default();
    let cost = price_transcription_job(pricing("job-1", 2, None), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, max_cost());
    assert_eq!(lookup.calls(), 2);
    assert_eq!(sleeper.slept(), vec![10.0_f64, 10.0_f64]);
}
