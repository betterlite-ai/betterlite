#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::worker_heartbeat::{
    HeartbeatDatabase, HeartbeatFuture, HostnameError, SystemWorkerIdentity, WorkerIdentity,
};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Upsert(String, String),
    Prune(String),
    Deregister(String),
    Count(String),
}

impl Call {
    fn failure(&self) -> FakeError {
        match self {
            Self::Upsert(..) => FakeError::Upsert,
            Self::Prune(..) => FakeError::Prune,
            Self::Deregister(..) => FakeError::Deregister,
            Self::Count(..) => FakeError::Count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FakeError {
    #[error("upsert failed")]
    Upsert,
    #[error("prune failed")]
    Prune,
    #[error("deregister failed")]
    Deregister,
    #[error("count failed")]
    Count,
}

#[derive(Default)]
pub(crate) struct FakeDatabase {
    calls: Mutex<Vec<Call>>,
    failing: Option<FakeError>,
    counts: Vec<Option<i32>>,
    gate: Option<Arc<Notify>>,
}

impl FakeDatabase {
    #[must_use]
    pub(crate) fn failing(mut self, failure: FakeError) -> Self {
        self.failing = Some(failure);
        self
    }

    #[must_use]
    pub(crate) fn with_counts(mut self, counts: Vec<Option<i32>>) -> Self {
        self.counts = counts;
        self
    }

    #[must_use]
    pub(crate) fn blocked(mut self) -> Self {
        self.gate = Some(Arc::new(Notify::new()));
        self
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        lock(&self.calls).clone()
    }

    fn record(&self, call: Call) {
        lock(&self.calls).push(call);
    }

    async fn settle(&self, failure: FakeError) -> Result<(), FakeError> {
        if self.failing == Some(failure) {
            return Err(failure);
        }
        if let Some(gate) = &self.gate {
            gate.notified().await;
        }
        Ok(())
    }

    fn answer(&self, call: Call) -> HeartbeatFuture<'_, (), FakeError> {
        let failure = call.failure();
        self.record(call);
        Box::pin(self.settle(failure))
    }
}

impl HeartbeatDatabase for FakeDatabase {
    type Error = FakeError;

    fn upsert<'database>(
        &'database self,
        worker_id: &'database str,
        hostname: &'database str,
    ) -> HeartbeatFuture<'database, (), FakeError> {
        self.answer(Call::Upsert(worker_id.to_owned(), hostname.to_owned()))
    }

    fn prune(&self, retention_seconds: f64) -> HeartbeatFuture<'_, (), FakeError> {
        self.answer(Call::Prune(retention_seconds.to_string()))
    }

    fn deregister<'database>(
        &'database self,
        worker_id: &'database str,
    ) -> HeartbeatFuture<'database, (), FakeError> {
        self.answer(Call::Deregister(worker_id.to_owned()))
    }

    fn count_live_workers(
        &self,
        window_seconds: f64,
    ) -> HeartbeatFuture<'_, Vec<Option<i32>>, FakeError> {
        self.record(Call::Count(window_seconds.to_string()));
        let counts = self.counts.clone();
        let failing = self.failing == Some(FakeError::Count);
        Box::pin(async move {
            if failing {
                return Err(FakeError::Count);
            }
            Ok(counts)
        })
    }
}

pub(crate) struct RecordingIdentity {
    hostname: &'static str,
    failing: bool,
    hostname_calls: AtomicUsize,
    generated_calls: AtomicUsize,
}

impl RecordingIdentity {
    pub(crate) fn new(hostname: &'static str) -> Self {
        Self {
            hostname,
            failing: false,
            hostname_calls: AtomicUsize::new(0),
            generated_calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn failing_hostname() -> Self {
        Self {
            hostname: "",
            failing: true,
            hostname_calls: AtomicUsize::new(0),
            generated_calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn hostname_calls(&self) -> usize {
        self.hostname_calls.load(Ordering::SeqCst)
    }

    pub(crate) fn generated_calls(&self) -> usize {
        self.generated_calls.load(Ordering::SeqCst)
    }
}

impl WorkerIdentity for RecordingIdentity {
    fn hostname(&self) -> Result<String, HostnameError> {
        self.hostname_calls.fetch_add(1, Ordering::SeqCst);
        if self.failing {
            return Err(HostnameError::Provider);
        }
        Ok(self.hostname.to_owned())
    }

    fn generate_worker_id(&self) -> String {
        self.generated_calls.fetch_add(1, Ordering::SeqCst);
        SystemWorkerIdentity.generate_worker_id()
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn contents(&self) -> String {
        let bytes = lock(&self.0).clone();
        String::from_utf8(bytes).expect("the log writer records UTF-8")
    }

    pub(crate) fn has(&self, level: &str, needle: &str) -> bool {
        self.contents()
            .lines()
            .any(|line| line.contains(level) && line.contains(needle))
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) struct LogCapture {
    pub(crate) logs: CapturedLogs,
    _guard: DefaultGuard,
}

pub(crate) fn capture_logs() -> LogCapture {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs.clone())
        .finish();
    let guard = set_default(subscriber);
    LogCapture {
        logs,
        _guard: guard,
    }
}
