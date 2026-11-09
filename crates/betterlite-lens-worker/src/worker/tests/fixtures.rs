#![cfg(test)]

use std::collections::VecDeque;
use std::future::ready;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_lens::analysis::Candidate;
use betterlite_lens::models::execution::{
    Coverage, Execution, ExecutionContent, ExecutionSource, Sample, TracePart,
};
use betterlite_lens::models::messages::{
    Claim, JobResult, ModelRequest, ModelResult, Progress, Purpose,
};
use serde::{Deserialize, Serialize};
use tokio::time::{Instant, sleep};

use crate::api::{LensApi, ResultStatus, WorkerError};

pub(super) fn claim() -> Claim {
    let mut claim: Claim = serde_json::from_str(include_str!("../../fixtures/claim.json")).unwrap();
    claim.job.settings.concurrency = 1;
    claim
}

pub(super) fn execution(id: &str) -> Execution {
    Execution {
        id: id.to_owned(),
        source: ExecutionSource::Traces,
        trace_id: String::from("trace"),
        trace_ref: String::new(),
        team_id: String::from("team"),
        name: String::from("run"),
        start_time: String::from("2026-01-01T00:00:00Z"),
        span_count: 1,
        root_seen: false,
        service: String::new(),
        metadata: Vec::new(),
    }
}

pub(super) fn sample() -> Sample {
    Sample {
        executions: vec![execution("exec")],
        eligible: 1,
        selected: 1,
        next_offset: None,
        next_cursor: None,
    }
}

pub(super) fn page() -> ExecutionContent {
    ExecutionContent {
        execution: execution("exec"),
        parts: vec![TracePart {
            execution_id: String::from("exec"),
            span_id: String::from("s0"),
            parent_span_id: String::new(),
            name: String::from("root"),
            kind: String::from("agent"),
            content: String::from("content"),
            truncated: false,
        }],
        next_cursor: None,
        partial: false,
    }
}

pub(super) fn model_result(content: &str) -> ModelResult {
    let escaped = serde_json::to_string(content).unwrap();
    let body = format!(r#"{{"content":{escaped},"cost":0}}"#);
    serde_json::from_str(&body).unwrap()
}

#[derive(Deserialize, Serialize)]
struct SentCandidates {
    candidates: Vec<Candidate>,
}

fn extraction_answer() -> String {
    String::from(
        r#"{"observations":[{"check_id":"expected_behavior","summary":"supported","evidence":[{"execution_id":"r0","span_id":"s0","quote":"content"}]}]}"#,
    )
}

fn investigation_answer() -> String {
    String::from(
        r#"{"action":"submit","finding":{"title":"Observed issue","description":"Found a concrete issue in the trace.","check_id":"expected_behavior","evidence":[{"execution_id":"r0","span_id":"s0","quote":"content"}]}}"#,
    )
}

fn default_model_answer(request: &ModelRequest) -> ModelResult {
    let content = match request.purpose {
        Purpose::Extract => extraction_answer(),
        Purpose::Cluster => {
            let sent: SentCandidates = serde_json::from_str(&request.prompt).unwrap();
            serde_json::to_string(&sent).unwrap()
        }
        Purpose::Investigate => investigation_answer(),
    };
    model_result(&content)
}

pub(super) struct FakeApi {
    pub(super) claim_calls: Mutex<Vec<Instant>>,
    pub(super) claimed: Mutex<VecDeque<Result<Option<Claim>, WorkerError>>>,
    pub(super) samples: Mutex<VecDeque<Result<Sample, WorkerError>>>,
    pub(super) pages: Mutex<VecDeque<Result<ExecutionContent, WorkerError>>>,
    pub(super) content_calls: Mutex<Vec<(String, String, i64)>>,
    pub(super) model_answers: Mutex<VecDeque<Result<ModelResult, WorkerError>>>,
    pub(super) model_calls: Mutex<Vec<(Instant, ModelRequest)>>,
    pub(super) progress_calls: Mutex<Vec<(String, Coverage)>>,
    pub(super) heartbeats: Mutex<Vec<Instant>>,
    pub(super) heartbeat_failure: Mutex<Option<WorkerError>>,
    pub(super) results: Mutex<Vec<JobResult>>,
    pub(super) result_answers: Mutex<VecDeque<Result<u16, WorkerError>>>,
    pub(super) sample_delay: Mutex<Duration>,
}

impl Default for FakeApi {
    fn default() -> Self {
        Self {
            claim_calls: Mutex::new(Vec::new()),
            claimed: Mutex::new(VecDeque::new()),
            samples: Mutex::new(VecDeque::new()),
            pages: Mutex::new(VecDeque::new()),
            content_calls: Mutex::new(Vec::new()),
            model_answers: Mutex::new(VecDeque::new()),
            model_calls: Mutex::new(Vec::new()),
            progress_calls: Mutex::new(Vec::new()),
            heartbeats: Mutex::new(Vec::new()),
            heartbeat_failure: Mutex::new(None),
            results: Mutex::new(Vec::new()),
            result_answers: Mutex::new(VecDeque::new()),
            sample_delay: Mutex::new(Duration::ZERO),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeApi {
    fn record_claim(&self) -> Result<Option<Claim>, WorkerError> {
        lock(&self.claim_calls).push(Instant::now());
        lock(&self.claimed).pop_front().unwrap_or(Ok(None))
    }

    fn record_content(&self, call: (String, String, i64)) -> Result<ExecutionContent, WorkerError> {
        lock(&self.content_calls).push(call);
        lock(&self.pages).pop_front().unwrap_or_else(|| Ok(page()))
    }

    fn record_model(&self, request: &ModelRequest) -> Result<ModelResult, WorkerError> {
        let fallback = default_model_answer(request);
        lock(&self.model_calls).push((Instant::now(), request.clone()));
        lock(&self.model_answers)
            .pop_front()
            .unwrap_or(Ok(fallback))
    }

    fn record_progress(&self, progress: Progress) {
        lock(&self.progress_calls).push((progress.stage, progress.coverage));
    }

    fn record_heartbeat(&self) -> Result<(), WorkerError> {
        lock(&self.heartbeats).push(Instant::now());
        lock(&self.heartbeat_failure).take().map_or(Ok(()), Err)
    }

    fn record_result(&self, result: &JobResult) -> Result<ResultStatus, WorkerError> {
        lock(&self.results).push(result.clone());
        let answer = lock(&self.result_answers).pop_front().unwrap_or(Ok(200));
        answer.map(|status| ResultStatus { status })
    }
}

impl LensApi for FakeApi {
    fn claim(&self) -> impl Future<Output = Result<Option<Claim>, WorkerError>> + Send {
        ready(self.record_claim())
    }

    async fn sample(&self, _prefix: &str) -> Result<Sample, WorkerError> {
        let delay = *lock(&self.sample_delay);
        sleep(delay).await;
        lock(&self.samples)
            .pop_front()
            .unwrap_or_else(|| Ok(sample()))
    }

    fn content(
        &self,
        _prefix: &str,
        execution_id: &str,
        cursor: &str,
        offset: i64,
    ) -> impl Future<Output = Result<ExecutionContent, WorkerError>> + Send {
        let call = (execution_id.to_owned(), cursor.to_owned(), offset);
        ready(self.record_content(call))
    }

    fn model(
        &self,
        _prefix: &str,
        request: ModelRequest,
    ) -> impl Future<Output = Result<ModelResult, WorkerError>> + Send {
        ready(self.record_model(&request))
    }

    fn progress(
        &self,
        _prefix: &str,
        progress: Progress,
    ) -> impl Future<Output = Result<(), WorkerError>> + Send {
        self.record_progress(progress);
        ready(Ok(()))
    }

    fn heartbeat(&self, _prefix: &str) -> impl Future<Output = Result<(), WorkerError>> + Send {
        ready(self.record_heartbeat())
    }

    fn result(
        &self,
        _prefix: &str,
        result: &JobResult,
    ) -> impl Future<Output = Result<ResultStatus, WorkerError>> + Send {
        ready(self.record_result(result))
    }
}
