use std::collections::{BTreeMap, VecDeque};
use std::future::{pending, ready};
use std::io::{Result as IoResult, Write};
use std::str::from_utf8;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::time::advance;

use tracing_subscriber::fmt::MakeWriter;

use crate::migration_ledger::ledger_reads::FailedLookup;
use crate::migration_ledger::lock::JitterSource;
use crate::migration_recovery::RecoveryError;
use crate::migration_recovery::StepCause;
use crate::migration_recovery::outcome::{DeployOutcome, FailedIdentity};
use crate::migration_recovery::policy::{RecoverySteps, RetryPause, StepFuture};

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn broken_cause() -> StepCause {
    let bytes = vec![255_u8];
    StepCause::Utf8(from_utf8(&bytes).expect_err("invalid UTF-8"))
}

pub(super) struct FakeJitter(pub(super) u64);

impl JitterSource for FakeJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        self.0.min(upper_exclusive_nanos.saturating_sub(1))
    }
}

#[derive(Default)]
pub(super) struct RecordingPause {
    delays: Mutex<Vec<Duration>>,
}

impl RecordingPause {
    pub(super) fn delays(&self) -> Vec<Duration> {
        lock(&self.delays).clone()
    }
}

impl RetryPause for RecordingPause {
    fn pause(&self, delay: Duration) -> StepFuture<'_, ()> {
        lock(&self.delays).push(delay);
        Box::pin(ready(()))
    }
}

#[derive(Default)]
pub(super) struct ScriptedSteps {
    deploys: Mutex<VecDeque<(DeployOutcome, Duration)>>,
    recoveries: Mutex<VecDeque<Result<bool, RecoveryError>>>,
    baselines: Mutex<VecDeque<Result<(), RecoveryError>>>,
    rollbacks: Mutex<VecDeque<Result<(), RecoveryError>>>,
    history: Mutex<BTreeMap<String, FailedLookup>>,
    timeouts: Mutex<Vec<Duration>>,
    events: Mutex<Vec<String>>,
    ahead: AtomicUsize,
    hostile: AtomicUsize,
    rollback_blocked: AtomicBool,
}

impl ScriptedSteps {
    pub(super) fn deploying(&self, outcome: DeployOutcome) {
        lock(&self.deploys).push_back((outcome, Duration::ZERO));
    }

    pub(super) fn deploying_after(&self, outcome: DeployOutcome, elapsed: Duration) {
        lock(&self.deploys).push_back((outcome, elapsed));
    }

    pub(super) fn recovering(&self, result: Result<bool, RecoveryError>) {
        lock(&self.recoveries).push_back(result);
    }

    pub(super) fn baselining(&self, result: Result<(), RecoveryError>) {
        lock(&self.baselines).push_back(result);
    }

    pub(super) fn history(&self, identity: &FailedIdentity, lookup: FailedLookup) {
        lock(&self.history).insert(identity_key(identity), lookup);
    }

    pub(super) fn report_hostile(&self, count: usize) {
        self.hostile.store(count, Ordering::SeqCst);
    }

    pub(super) fn rollback_fails(&self, error: RecoveryError) {
        lock(&self.rollbacks).push_back(Err(error));
    }

    pub(super) fn block_rollback(&self) {
        self.rollback_blocked.store(true, Ordering::SeqCst);
    }

    pub(super) fn events(&self) -> Vec<String> {
        lock(&self.events).clone()
    }

    pub(super) fn deploy_timeouts(&self) -> Vec<Duration> {
        lock(&self.timeouts).clone()
    }

    pub(super) fn ahead_checks(&self) -> usize {
        self.ahead.load(Ordering::SeqCst)
    }

    fn record(&self, event: String) {
        lock(&self.events).push(event);
    }
}

fn identity_key(identity: &FailedIdentity) -> String {
    format!("{}@{}", identity.name, identity.started_at)
}

impl RecoverySteps for ScriptedSteps {
    fn deploy(&self, timeout: Duration) -> StepFuture<'_, Result<DeployOutcome, RecoveryError>> {
        self.record("deploy".to_owned());
        lock(&self.timeouts).push(timeout);
        let (outcome, elapsed) = lock(&self.deploys)
            .pop_front()
            .expect("the test scripted a deploy outcome");
        if elapsed.is_zero() {
            return Box::pin(ready(Ok(outcome)));
        }
        Box::pin(async move {
            advance(elapsed).await;
            Ok(outcome)
        })
    }

    fn recover_completed<'step>(
        &'step self,
        name: &'step str,
    ) -> StepFuture<'step, Result<bool, RecoveryError>> {
        self.record(format!("recover:{name}"));
        let result = lock(&self.recoveries).pop_front().unwrap_or(Ok(false));
        Box::pin(ready(result))
    }

    fn baseline(&self) -> StepFuture<'_, Result<(), RecoveryError>> {
        self.record("baseline".to_owned());
        let result = lock(&self.baselines).pop_front().unwrap_or(Ok(()));
        Box::pin(ready(result))
    }

    fn failed_history<'step>(
        &'step self,
        identity: &'step FailedIdentity,
    ) -> StepFuture<'step, FailedLookup> {
        self.record(format!("history:{}", identity_key(identity)));
        let lookup = lock(&self.history)
            .remove(&identity_key(identity))
            .unwrap_or(FailedLookup::Absent);
        Box::pin(ready(lookup))
    }

    fn roll_back<'step>(
        &'step self,
        name: &'step str,
    ) -> StepFuture<'step, Result<(), RecoveryError>> {
        self.record(format!("rollback:{name}"));
        let scripted_result = lock(&self.rollbacks).pop_front();
        if let Some(scripted) = scripted_result {
            return Box::pin(ready(scripted));
        }
        if self.rollback_blocked.load(Ordering::SeqCst) {
            return Box::pin(pending());
        }
        Box::pin(ready(Ok(())))
    }

    fn warn_ahead(&self) -> StepFuture<'_, Vec<String>> {
        self.record("ahead".to_owned());
        self.ahead.fetch_add(1, Ordering::SeqCst);
        let count = self.hostile.load(Ordering::SeqCst);
        let names = (0..count)
            .map(|index| format!("20990101000000_ahead{index}"))
            .collect();
        Box::pin(ready(names))
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(super) fn contents(&self) -> String {
        let bytes = lock(&self.0).clone();
        String::from_utf8(bytes).expect("the log writer records UTF-8")
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

pub(super) fn capture_logs() -> (CapturedLogs, impl tracing::Subscriber + Send + Sync) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}
