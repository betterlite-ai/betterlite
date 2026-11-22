#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, Cursor, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use indexmap::IndexMap;
use tokio::sync::{Barrier, Notify};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::api::file_batch_guardrails::SCAN_WINDOW;
use crate::api::file_batch_guardrails::hook::{
    GuardrailsOnlyPreCall, PreCallInput, RaisedGuardrail,
};
use crate::api::file_batch_guardrails::record_json::text::RecordText;
use crate::api::file_batch_guardrails::record_json::{RecordJson, RecordObject};
use crate::api::file_batch_guardrails::records::{BatchScanResult, RecordChange};
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

pub(super) async fn scan_all<Source, Hook>(
    source: &mut Source,
    hook: &Hook,
    spool: &Spool,
    tasks: &TaskTracker,
) -> Result<ScanFile<Spool, Hook::Failure>, ScanFileError>
where
    Source: BufRead + Seek + Send,
    Hook: GuardrailsOnlyPreCall,
{
    let metadata = IndexMap::new();
    let key = principal();
    let catalog = route_catalog();
    let call = BatchScanCall {
        file_source: source,
        request_metadata: &metadata,
        user_api_key_dict: &key,
        routes: &catalog,
        proxy_logging_obj: hook,
        tasks,
        redactions: spool.clone(),
    };
    scan_batch_input_file(call).await
}

pub(super) async fn timed_scan<Source, Hook>(
    source: &mut Source,
    hook: &Hook,
    spool: &Spool,
    tasks: &TaskTracker,
) -> Result<ScanFile<Spool, Hook::Failure>, ScanFileError>
where
    Source: BufRead + Seek + Send,
    Hook: GuardrailsOnlyPreCall,
{
    timeout(TEST_TIMEOUT, scan_all(source, hook, spool, tasks))
        .await
        .expect("the scan did not finish in time")
}

// A seekable upload body that counts the block reads the scan makes and records every seek it asks for
pub(super) struct CountingSource {
    inner: Cursor<Vec<u8>>,
    pub(super) reads: Arc<AtomicUsize>,
    pub(super) seeks: Arc<Mutex<Vec<SeekFrom>>>,
}

impl CountingSource {
    pub(super) fn new(text: String) -> Self {
        Self {
            inner: Cursor::new(text.into_bytes()),
            reads: Arc::new(AtomicUsize::new(0)),
            seeks: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Read for CountingSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl BufRead for CountingSource {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.inner.consume(amount);
    }

    fn read_until(&mut self, delimiter: u8, buffer: &mut Vec<u8>) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read_until(delimiter, buffer)
    }
}

impl Seek for CountingSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks.lock().unwrap().push(position);
        self.inner.seek(position)
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
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks.lock().unwrap().push(position);
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
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
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
    FakeFailure {
        raised: Raised::Verdict {
            blocked: true,
            caused: false,
        },
        name: Some(name.to_owned()),
        detail_name: None,
        status: 400,
    }
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
    (ordinal.div_ceil(SCAN_WINDOW) * SCAN_WINDOW).min(rows)
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
    pub(super) finished: Mutex<Vec<String>>,
}

impl Probe {
    pub(super) fn new(reads: Arc<AtomicUsize>) -> Self {
        Self {
            reads,
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            started: Mutex::new(Vec::new()),
            finished: Mutex::new(Vec::new()),
        }
    }

    fn begin(&self, data: &RecordObject) -> ActiveCall<'_> {
        let active_before = self.active.fetch_add(1, Ordering::SeqCst);
        self.max_active
            .fetch_max(active_before + 1, Ordering::SeqCst);
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
    Raise(FakeFailure),
    Block(FakeFailure),
}

pub(super) struct ScriptedHook {
    answers: BTreeMap<String, Answer>,
    pub(super) probe: Probe,
}

impl ScriptedHook {
    pub(super) fn new(probe: Probe) -> Self {
        Self {
            answers: BTreeMap::new(),
            probe,
        }
    }

    pub(super) fn with(answers: Vec<(&str, Answer)>, probe: Probe) -> Self {
        Self {
            answers: answers
                .into_iter()
                .map(|(marker, answer)| (marker.to_owned(), answer))
                .collect(),
            probe,
        }
    }
}

#[async_trait]
impl GuardrailsOnlyPreCall for ScriptedHook {
    type Failure = FakeFailure;

    async fn pre_call_hook(&self, input: PreCallInput<'_>) -> Result<RecordObject, FakeFailure> {
        let call = self.probe.begin(&input.data);
        match self.answers.get(&call.marker) {
            Some(Answer::Rewrite) => {
                let mut data = input.data;
                data.insert(RecordText::from("rewritten"), RecordJson::Bool(true));
                Ok(data)
            }
            Some(Answer::Raise(failure) | Answer::Block(failure)) => Err(failure.clone()),
            None => Ok(input.data),
        }
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

// Every call waits at one barrier, so a test observes which calls started before any of them answered
pub(super) struct BarrierHook {
    pub(super) probe: Probe,
    pub(super) gate: Barrier,
    pub(super) failure: FakeFailure,
    pub(super) fail_marker: String,
}

#[async_trait]
impl GuardrailsOnlyPreCall for BarrierHook {
    type Failure = FakeFailure;

    async fn pre_call_hook(&self, input: PreCallInput<'_>) -> Result<RecordObject, FakeFailure> {
        let call = self.probe.begin(&input.data);
        self.gate.wait().await;
        if call.marker == self.fail_marker {
            return Err(self.failure.clone());
        }
        Ok(input.data)
    }
}

// Each window's calls finish in reverse row order, the hardest order for the scan to keep straight
pub(super) struct ReverseHook {
    pub(super) probe: Probe,
    done: Mutex<BTreeSet<usize>>,
    rows: usize,
}

impl ReverseHook {
    pub(super) fn new(probe: Probe, rows: usize) -> Self {
        Self {
            probe,
            done: Mutex::new(BTreeSet::new()),
            rows,
        }
    }

    fn later_rows_done(&self, ordinal: usize) -> bool {
        let end = window_end(ordinal, self.rows);
        let done = self.done.lock().unwrap();
        (ordinal + 1..=end).all(|later| done.contains(&later))
    }
}

#[async_trait]
impl GuardrailsOnlyPreCall for ReverseHook {
    type Failure = FakeFailure;

    async fn pre_call_hook(&self, input: PreCallInput<'_>) -> Result<RecordObject, FakeFailure> {
        let call = self.probe.begin(&input.data);
        let ordinal = ordinal_of(&call.marker);
        while !self.later_rows_done(ordinal) {
            tokio::task::yield_now().await;
        }
        self.done.lock().unwrap().insert(ordinal);
        let mut data = input.data;
        data.insert(RecordText::from("rewritten"), RecordJson::Bool(true));
        Ok(data)
    }
}

pub(super) struct CancelSignal(pub(super) Arc<AtomicUsize>);

impl Drop for CancelSignal {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// Every call starts and stays pending, and each dropped call says so, which is what cancellation leaves behind
pub(super) struct CancellingHook {
    pub(super) probe: Probe,
    pub(super) reached: Arc<Notify>,
    pub(super) cancelled: Arc<AtomicUsize>,
}

impl CancellingHook {
    pub(super) fn new(probe: Probe, reached: Arc<Notify>, cancelled: Arc<AtomicUsize>) -> Self {
        Self {
            probe,
            reached,
            cancelled,
        }
    }
}

#[async_trait]
impl GuardrailsOnlyPreCall for CancellingHook {
    type Failure = FakeFailure;

    async fn pre_call_hook(&self, input: PreCallInput<'_>) -> Result<RecordObject, FakeFailure> {
        let _call = self.probe.begin(&input.data);
        let _cancel = CancelSignal(Arc::clone(&self.cancelled));
        if self.probe.started.lock().unwrap().len() == SCAN_WINDOW {
            self.reached.notify_one();
        }
        futures_util::future::pending::<()>().await;
        Ok(input.data)
    }
}
