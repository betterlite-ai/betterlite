#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, Cursor, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::ThreadId;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use futures_util::future::pending;
use indexmap::IndexMap;
use tokio::sync::{Barrier, Notify};
use tokio::task::yield_now;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::api::file_batch_guardrails::SCAN_WINDOW;
use crate::api::file_batch_guardrails::hook::{
    GuardrailsOnlyPreCall, PreCallInput, RaisedGuardrail,
};
use crate::api::file_batch_guardrails::record_json::text::RecordText;
use crate::api::file_batch_guardrails::record_json::{RecordJson, RecordObject};
use crate::api::file_batch_guardrails::records::{BatchScanResult, RecordChange};
use crate::api::file_batch_rewrite::body::UploadBody;
use crate::api::file_batch_rewrite::error::ScanFileError;
use crate::api::file_batch_rewrite::scan::{BatchScanCall, ScanFile, scan_batch_input_file};
use crate::api::file_batch_rewrite::spool::Spool;
use crate::routes::catalog::{CATALOG, RouteCatalog};

pub(super) const TEST_TIMEOUT: Duration = Duration::from_secs(10);

const UNREADABLE: &str = "the upload body cannot be read";

pub(super) fn route_catalog() -> RouteCatalog {
    RouteCatalog::parse(CATALOG).unwrap()
}

pub(super) fn principal() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap()
}

// One valid scan row per line, with its custom_id and marker both naming the physical line
pub(super) fn lines_of(count: usize) -> String {
    (1..=count)
        .map(|line| {
            format!(r#"{{"custom_id":"r{line}","body":{{"marker":"r{line}","messages":[]}}}}"#)
        })
        .collect::<Vec<String>>()
        .join("\n")
}

pub(super) fn scan_result(
    changes: Vec<RecordChange>,
    scanned_records: usize,
    redactions: Spool,
) -> BatchScanResult<Spool> {
    BatchScanResult {
        changes,
        scanned_records,
        redactions,
    }
}

pub(super) fn line_of_change(change: &RecordChange) -> usize {
    match change {
        RecordChange::Redacted(record) => record.line_number,
        RecordChange::Dropped(record) => record.line_number,
    }
}

pub(super) async fn timed_scan<Source, Hook>(
    body: &UploadBody<Source>,
    hook: &Hook,
    spool: &Spool,
    tasks: &TaskTracker,
) -> Result<ScanFile<Spool, Hook::Failure>, ScanFileError>
where
    Source: BufRead + Seek + Send + 'static,
    Hook: GuardrailsOnlyPreCall,
{
    let metadata = IndexMap::new();
    let key = principal();
    let catalog = route_catalog();
    let call = BatchScanCall {
        file_source: body.clone(),
        request_metadata: &metadata,
        user_api_key_dict: &key,
        routes: &catalog,
        proxy_logging_obj: hook,
        tasks,
        redactions: spool.clone(),
    };
    timeout(TEST_TIMEOUT, scan_batch_input_file(call))
        .await
        .expect("the scan did not finish in time")
}

// A seekable upload body that counts the block reads the scan makes and records every seek and reading thread it asks for
pub(super) struct CountingSource {
    inner: Cursor<Vec<u8>>,
    pub(super) reads: Arc<AtomicUsize>,
    pub(super) seeks: Arc<Mutex<Vec<SeekFrom>>>,
    pub(super) threads: Arc<Mutex<Vec<ThreadId>>>,
}

impl CountingSource {
    pub(super) fn new(text: String) -> Self {
        Self {
            inner: Cursor::new(text.into_bytes()),
            reads: Arc::new(AtomicUsize::new(0)),
            seeks: Arc::new(Mutex::new(Vec::new())),
            threads: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Read for CountingSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl BufRead for CountingSource {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.inner.consume(amount);
    }

    fn read_until(&mut self, byte: u8, buf: &mut Vec<u8>) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ThreadId::current());
        self.inner.read_until(byte, buf)
    }
}

impl Seek for CountingSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.seeks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(pos);
        self.inner.seek(pos)
    }
}

// A counting source already handed to the blocking port, with its read, seek and thread tallies kept reachable
pub(super) struct CountedBody {
    pub(super) body: UploadBody<CountingSource>,
    pub(super) reads: Arc<AtomicUsize>,
    pub(super) seeks: Arc<Mutex<Vec<SeekFrom>>>,
    pub(super) threads: Arc<Mutex<Vec<ThreadId>>>,
}

impl CountedBody {
    pub(super) fn new(text: String) -> Self {
        let source = CountingSource::new(text);
        let reads = Arc::clone(&source.reads);
        let seeks = Arc::clone(&source.seeks);
        let threads = Arc::clone(&source.threads);
        Self {
            body: UploadBody::new(source),
            reads,
            seeks,
            threads,
        }
    }
}

// An upload body whose every read fails, so a run reaches the file after the scan accepted it
#[derive(Default)]
pub(super) struct FailingSource {
    pub(super) seeks: Arc<Mutex<Vec<SeekFrom>>>,
}

impl Read for FailingSource {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other(UNREADABLE))
    }
}

impl BufRead for FailingSource {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::Error::other(UNREADABLE))
    }

    fn consume(&mut self, _amount: usize) {}
}

impl Seek for FailingSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.seeks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(pos);
        Ok(0)
    }
}

// An upload body that reads fine and refuses to rewind
pub(super) struct UnrewindableSource {
    inner: Cursor<Vec<u8>>,
}

impl UnrewindableSource {
    pub(super) fn new(text: String) -> Self {
        Self {
            inner: Cursor::new(text.into_bytes()),
        }
    }
}

impl Read for UnrewindableSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl BufRead for UnrewindableSource {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.inner.consume(amount);
    }
}

impl Seek for UnrewindableSource {
    fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
        Err(io::Error::other("the upload body refuses to rewind"))
    }
}

// A GuardrailRaisedException says whether it blocked; anything else only carries a status and maybe a cause
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Raised {
    Verdict { blocked: bool, caused: bool },
    Status { intervention: bool, caused: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FakeFailure {
    pub(super) raised: Raised,
    pub(super) name: Option<String>,
    pub(super) detail_name: Option<String>,
    pub(super) status: u16,
}

pub(super) fn raised(name: &str, status: u16) -> FakeFailure {
    FakeFailure {
        raised: Raised::Status {
            intervention: false,
            caused: false,
        },
        name: Some(name.to_owned()),
        detail_name: None,
        status,
    }
}

pub(super) fn blocked(name: &str) -> FakeFailure {
    let mut failure = raised(name, 400);
    failure.raised = Raised::Verdict {
        blocked: true,
        caused: false,
    };
    failure
}

impl RaisedGuardrail for FakeFailure {
    fn blocked_content(&self) -> Option<bool> {
        match self.raised {
            Raised::Verdict { blocked, .. } => Some(blocked),
            Raised::Status { .. } => None,
        }
    }

    fn has_cause(&self) -> bool {
        match self.raised {
            Raised::Verdict { caused, .. } | Raised::Status { caused, .. } => caused,
        }
    }

    fn is_intervention(&self) -> bool {
        match self.raised {
            Raised::Verdict { .. } => true,
            Raised::Status { intervention, .. } => intervention,
        }
    }

    fn guardrail_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn detail_guardrail_name(&self) -> Option<&str> {
        self.detail_name.as_deref()
    }
}

pub(super) fn marker_of(data: &RecordObject) -> String {
    data.get(&RecordText::from("marker"))
        .and_then(RecordJson::as_text)
        .map(|text| text.replaced('?'))
        .unwrap_or_default()
}

pub(super) fn ordinal_of(marker: &str) -> usize {
    marker.trim_start_matches('r').parse().unwrap_or(0)
}

fn window_end(ordinal: usize, rows: usize) -> usize {
    ordinal
        .div_ceil(SCAN_WINDOW)
        .saturating_mul(SCAN_WINDOW)
        .min(rows)
}

#[derive(Debug)]
pub(super) struct Started {
    pub(super) marker: String,
    pub(super) active_before: usize,
    pub(super) reads_before: usize,
}

// What each hook call saw when it started: how many siblings were in flight and how many lines the scan had read
#[derive(Debug)]
pub(super) struct Probe {
    reads: Arc<AtomicUsize>,
    pub(super) active: AtomicUsize,
    pub(super) max_active: AtomicUsize,
    pub(super) started: Mutex<Vec<Started>>,
    pub(super) completed: Mutex<Vec<String>>,
    pub(super) finished: Mutex<Vec<String>>,
}

impl Probe {
    pub(super) fn new(reads: Arc<AtomicUsize>) -> Self {
        Self {
            reads,
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            started: Mutex::new(Vec::new()),
            completed: Mutex::new(Vec::new()),
            finished: Mutex::new(Vec::new()),
        }
    }

    fn begin(&self, data: &RecordObject) -> ActiveCall<'_> {
        let active_before = self.active.fetch_add(1, Ordering::SeqCst);
        self.max_active
            .fetch_max(active_before.saturating_add(1), Ordering::SeqCst);
        let marker = marker_of(data);
        let reads_before = self.reads.load(Ordering::SeqCst);
        self.started.lock().unwrap().push(Started {
            marker: marker.clone(),
            active_before,
            reads_before,
        });
        ActiveCall {
            probe: self,
            marker,
        }
    }
}

pub(super) struct ActiveCall<'probe> {
    probe: &'probe Probe,
    pub(super) marker: String,
}

impl Drop for ActiveCall<'_> {
    fn drop(&mut self) {
        self.probe.active.fetch_sub(1, Ordering::SeqCst);
        self.probe
            .finished
            .lock()
            .unwrap()
            .push(self.marker.clone());
    }
}

pub(super) enum Answer {
    Rewrite,
    Replace(RecordObject),
    Raise(FakeFailure),
    Block(FakeFailure),
    // Raise only after the call for the named marker has returned, so a slow earlier line follows a fast later one
    RaiseAfter(FakeFailure, String),
}

// What the hook does besides answering by marker: finish each window in reverse row order, or stay pending until cancelled
enum Behavior {
    Answers,
    Reverse(Reverse),
    Cancel(CancelWatch),
}

struct Reverse {
    done: Mutex<BTreeSet<usize>>,
    rows: usize,
}

impl Reverse {
    fn new(rows: usize) -> Self {
        Self {
            done: Mutex::new(BTreeSet::new()),
            rows,
        }
    }

    fn later_rows_done(&self, ordinal: usize) -> bool {
        let end = window_end(ordinal, self.rows);
        let done = self.done.lock().unwrap();
        (ordinal.saturating_add(1)..=end).all(|later| done.contains(&later))
    }
}

// Every dropped call says so, which is what cancellation leaves behind
struct CancelWatch {
    reached: Arc<Notify>,
    cancelled: Arc<AtomicUsize>,
}

impl CancelWatch {
    fn arm(&self, probe: &Probe) -> CancelSignal {
        if probe.started.lock().unwrap().len() == SCAN_WINDOW {
            self.reached.notify_one();
        }
        CancelSignal(Arc::clone(&self.cancelled))
    }
}

pub(super) struct ScriptedHook {
    answers: BTreeMap<String, Answer>,
    behavior: Behavior,
    gate: Option<Barrier>,
    pub(super) probe: Probe,
}

impl ScriptedHook {
    pub(super) fn new(probe: Probe) -> Self {
        Self::with(Vec::new(), probe)
    }

    pub(super) fn with(answers: Vec<(&str, Answer)>, probe: Probe) -> Self {
        Self {
            answers: answers
                .into_iter()
                .map(|(marker, answer)| (marker.to_owned(), answer))
                .collect(),
            behavior: Behavior::Answers,
            gate: None,
            probe,
        }
    }

    pub(super) fn reversing(probe: Probe, rows: usize) -> Self {
        Self::with(Vec::new(), probe).in_behavior(Behavior::Reverse(Reverse::new(rows)))
    }

    pub(super) fn cancelling(
        probe: Probe,
        reached: Arc<Notify>,
        cancelled: Arc<AtomicUsize>,
    ) -> Self {
        let watch = CancelWatch { reached, cancelled };
        Self::with(Vec::new(), probe).in_behavior(Behavior::Cancel(watch))
    }

    pub(super) fn behind_gate(mut self, gate: Barrier) -> Self {
        self.gate = Some(gate);
        self
    }

    fn in_behavior(mut self, behavior: Behavior) -> Self {
        self.behavior = behavior;
        self
    }
}

fn rewritten(mut data: RecordObject) -> RecordObject {
    data.insert(RecordText::from("rewritten"), RecordJson::Bool(true));
    data
}

#[async_trait]
impl GuardrailsOnlyPreCall for ScriptedHook {
    type Failure = FakeFailure;

    async fn pre_call_hook(&self, input: PreCallInput<'_>) -> Result<RecordObject, FakeFailure> {
        let call = self.probe.begin(&input.data);
        if let Some(gate) = &self.gate {
            gate.wait().await;
        }
        let answer = match &self.behavior {
            Behavior::Answers => match self.answers.get(&call.marker) {
                Some(Answer::Rewrite) => Ok(rewritten(input.data)),
                Some(Answer::Replace(data)) => Ok(data.clone()),
                Some(Answer::Raise(failure) | Answer::Block(failure)) => Err(failure.clone()),
                Some(Answer::RaiseAfter(failure, after)) => {
                    while !self
                        .probe
                        .completed
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|done| done == after)
                    {
                        yield_now().await;
                    }
                    Err(failure.clone())
                }
                None => Ok(input.data),
            },
            Behavior::Reverse(reverse) => {
                let ordinal = ordinal_of(&call.marker);
                while !reverse.later_rows_done(ordinal) {
                    yield_now().await;
                }
                reverse.done.lock().unwrap().insert(ordinal);
                Ok(rewritten(input.data))
            }
            Behavior::Cancel(watch) => {
                let _signal = watch.arm(&self.probe);
                pending::<()>().await;
                Ok(input.data)
            }
        };
        self.probe
            .completed
            .lock()
            .unwrap()
            .push(call.marker.clone());
        answer
    }
}

// Rewrites every record, which makes a scan spool one redaction per row
pub(super) struct RewriteAll;

#[async_trait]
impl GuardrailsOnlyPreCall for RewriteAll {
    type Failure = FakeFailure;

    async fn pre_call_hook(&self, input: PreCallInput<'_>) -> Result<RecordObject, FakeFailure> {
        let mut data = input.data;
        data.insert(RecordText::from("rewritten"), RecordJson::Bool(true));
        Ok(data)
    }
}

pub(super) struct CancelSignal(Arc<AtomicUsize>);

impl Drop for CancelSignal {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
