#![cfg(test)]

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rstest::rstest;
use tokio::time::timeout;

use crate::polling::{
    PollOutcome, PollSleeper, TokioSleeper, await_transcription_job, aws_error_type,
};
use crate::records::{
    GetTranscriptionJobRequest, JobLookup, JobLookupError, JobLookupFuture, TranscriptionJobRecord,
    TranscriptionJobResponse,
};

fn record(status: &str) -> TranscriptionJobRecord {
    TranscriptionJobRecord {
        transcription_job_status: Some(status.to_owned()),
        creation_time: None,
        media: None,
        tags: Vec::new(),
    }
}

#[derive(Debug)]
enum Reply {
    Status(&'static str),
    NoJob,
    Rejected(u16, Option<&'static str>),
    Unavailable,
    Invalid,
}

struct ScriptedLookup {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<usize>,
}

impl ScriptedLookup {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl JobLookup for ScriptedLookup {
    fn get_transcription_job<'lookup>(
        &'lookup self,
        _request: &'lookup GetTranscriptionJobRequest,
    ) -> JobLookupFuture<'lookup> {
        Box::pin(async move {
            let mut calls = self.calls.lock().unwrap();
            *calls = calls.saturating_add(1);
            drop(calls);
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Reply::Unavailable);
            match reply {
                Reply::Status(status) => Ok(TranscriptionJobResponse {
                    transcription_job: Some(record(status)),
                }),
                Reply::NoJob => Ok(TranscriptionJobResponse {
                    transcription_job: None,
                }),
                Reply::Rejected(status, error_type) => Err(JobLookupError::Rejected {
                    status,
                    aws_error_type: error_type.map(str::to_owned),
                }),
                Reply::Unavailable => Err(JobLookupError::Unavailable),
                Reply::Invalid => Err(JobLookupError::InvalidResponse(
                    serde_json::from_str::<TranscriptionJobResponse>("{").unwrap_err(),
                )),
            }
        })
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

async fn poll_status(status: Option<&'static str>) -> Option<PollOutcome> {
    let lookup = ScriptedLookup::new(vec![status.map_or(Reply::NoJob, Reply::Status)]);
    await_transcription_job("job-1", &lookup, &RecordingSleeper::default(), 1).await
}

#[rstest]
#[case::completed("COMPLETED")]
#[case::failed("FAILED")]
#[tokio::test]
async fn poll_status_accepts_terminal_statuses(#[case] status: &'static str) {
    assert_eq!(
        poll_status(Some(status)).await,
        Some(PollOutcome::Finished(record(status)))
    );
}

#[rstest]
#[case::running("RUNNING")]
#[case::queued("QUEUED")]
#[case::in_progress("IN_PROGRESS")]
#[case::lowercase("completed")]
#[case::empty("")]
#[tokio::test]
async fn poll_status_treats_every_other_status_as_unfinished(#[case] status: &'static str) {
    assert_eq!(poll_status(Some(status)).await, None);
}

#[tokio::test]
async fn poll_status_treats_a_missing_job_as_unfinished() {
    assert_eq!(poll_status(None).await, None);
}

#[rstest]
#[case::hash_prefix(
    b"{\"__type\":\"com.amazon.coral.service#NotFoundException\"}",
    Some("NotFoundException")
)]
#[case::no_hash(b"{\"__type\":\"BadRequestException\"}", Some("BadRequestException"))]
#[case::many_hashes(b"{\"__type\":\"a#b#c\"}", Some("c"))]
#[case::empty_type(b"{\"__type\":\"\"}", Some(""))]
fn error_type_reads_the_member_after_the_last_hash(
    #[case] body: &[u8],
    #[case] expected: Option<&str>,
) {
    assert_eq!(aws_error_type(body).as_deref(), expected);
}

#[rstest]
#[case::plain_text(b"not json")]
#[case::half_object(b"{\"__type\":")]
#[case::array(b"[1,2]")]
#[case::array_of_the_type_string(b"[\"com.amazon.coral.service#NotFoundException\"]")]
#[case::number(b"42")]
#[case::string(b"\"text\"")]
#[case::null(b"null")]
#[case::empty(b"")]
#[case::invalid_utf8(b"{\xFF}")]
fn error_type_is_none_for_a_body_that_is_not_a_json_object(#[case] body: &[u8]) {
    assert_eq!(aws_error_type(body), None);
}

#[rstest]
#[case::missing(b"{}")]
#[case::null(b"{\"__type\":null}")]
#[case::number(b"{\"__type\":5}")]
#[case::object(b"{\"__type\":{}}")]
#[case::array(b"{\"__type\":[]}")]
#[case::other_members(b"{\"message\":\"gone\"}")]
fn error_type_is_none_without_a_string_type(#[case] body: &[u8]) {
    assert_eq!(aws_error_type(body), None);
}

async fn poll_outcome(reply: Reply) -> Option<PollOutcome> {
    let lookup = ScriptedLookup::new(vec![reply]);
    await_transcription_job("job-1", &lookup, &RecordingSleeper::default(), 1).await
}

#[rstest]
#[case::not_found(Reply::Rejected(400, Some("NotFoundException")), true)]
#[case::bad_request(Reply::Rejected(400, Some("BadRequestException")), true)]
#[case::internal(Reply::Rejected(500, Some("InternalFailure")), false)]
#[case::throttled(Reply::Rejected(400, Some("ThrottlingException")), false)]
#[case::no_type(Reply::Rejected(400, None), false)]
#[case::unavailable(Reply::Unavailable, false)]
#[case::invalid(Reply::Invalid, false)]
#[tokio::test]
async fn missing_job_turns_only_deleted_job_errors_into_missing(
    #[case] reply: Reply,
    #[case] missing: bool,
) {
    let outcome = poll_outcome(reply).await;
    if missing {
        assert_eq!(outcome, Some(PollOutcome::Missing));
    } else {
        assert_eq!(outcome, None);
    }
}

#[tokio::test]
async fn await_returns_a_finished_job_without_sleeping() {
    let lookup = ScriptedLookup::new(vec![Reply::Status("COMPLETED")]);
    let sleeper = RecordingSleeper::default();
    let outcome = await_transcription_job("job-1", &lookup, &sleeper, 5).await;
    assert_eq!(outcome, Some(PollOutcome::Finished(record("COMPLETED"))));
    assert_eq!(lookup.calls(), 1);
    assert_eq!(sleeper.slept(), Vec::<f64>::new());
}

#[tokio::test]
async fn await_returns_a_missing_job_without_sleeping() {
    let lookup = ScriptedLookup::new(vec![Reply::Rejected(400, Some("NotFoundException"))]);
    let sleeper = RecordingSleeper::default();
    let outcome = await_transcription_job("job-1", &lookup, &sleeper, 5).await;
    assert_eq!(outcome, Some(PollOutcome::Missing));
    assert_eq!(lookup.calls(), 1);
    assert_eq!(sleeper.slept(), Vec::<f64>::new());
}

#[tokio::test]
async fn await_sleeps_after_every_unfinished_attempt_including_the_last() {
    let lookup = ScriptedLookup::new(vec![
        Reply::Status("RUNNING"),
        Reply::Status("RUNNING"),
        Reply::Status("RUNNING"),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = await_transcription_job("job-1", &lookup, &sleeper, 3).await;
    assert_eq!(outcome, None);
    assert_eq!(lookup.calls(), 3);
    assert_eq!(sleeper.slept(), vec![10.0_f64, 10.0_f64, 10.0_f64]);
}

#[tokio::test]
async fn await_finishes_on_the_last_allowed_attempt() {
    let lookup = ScriptedLookup::new(vec![
        Reply::Status("RUNNING"),
        Reply::Status("RUNNING"),
        Reply::Status("FAILED"),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = await_transcription_job("job-1", &lookup, &sleeper, 3).await;
    assert_eq!(outcome, Some(PollOutcome::Finished(record("FAILED"))));
    assert_eq!(lookup.calls(), 3);
    assert_eq!(sleeper.slept(), vec![10.0_f64, 10.0_f64]);
}

#[tokio::test]
async fn await_makes_no_attempt_when_the_limit_is_zero() {
    let lookup = ScriptedLookup::new(vec![Reply::Status("COMPLETED")]);
    let sleeper = RecordingSleeper::default();
    let outcome = await_transcription_job("job-1", &lookup, &sleeper, 0).await;
    assert_eq!(outcome, None);
    assert_eq!(lookup.calls(), 0);
    assert_eq!(sleeper.slept(), Vec::<f64>::new());
}

#[tokio::test]
async fn await_stops_at_the_attempt_limit() {
    let lookup = ScriptedLookup::new(vec![
        Reply::Unavailable,
        Reply::Unavailable,
        Reply::Status("COMPLETED"),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = await_transcription_job("job-1", &lookup, &sleeper, 2).await;
    assert_eq!(outcome, None);
    assert_eq!(lookup.calls(), 2);
    assert_eq!(sleeper.slept(), vec![10.0_f64, 10.0_f64]);
}

#[tokio::test]
async fn await_tokio_sleeper_waits_for_the_requested_seconds() {
    let started = Instant::now();
    TokioSleeper.sleep(0.05_f64).await;
    assert!(started.elapsed() >= Duration::from_millis(50));
}

#[rstest]
#[case::negative(-1.0_f64)]
#[case::not_a_number(f64::NAN)]
#[tokio::test]
async fn await_tokio_sleeper_returns_at_once_for_a_delay_asyncio_would_skip(#[case] seconds: f64) {
    let outcome = timeout(Duration::from_secs(5), TokioSleeper.sleep(seconds)).await;
    outcome.unwrap();
}
