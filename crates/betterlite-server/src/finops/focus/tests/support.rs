#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::{Future, pending, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use betterlite_logging::finops::focus::destinations::factory::resolve::DestinationOverrides;
use betterlite_logging::finops::focus::engine::ExportSettings;
use betterlite_logging::finops::metering::clock::UtcClock;
use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry;

use super::export::{
    FocusEngineFactory, FocusExporter, FocusLogger, FocusLoggerDeps, FocusLoggerParams,
};
use super::export_schedule::{FocusPodLock, FocusScheduler, ScheduleTrigger};

pub(super) const FIXED_NOW: &str = "2024-03-10T13:47:23.123456Z";

pub(super) fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("the test timestamp is valid RFC 3339")
        .to_utc()
}

pub(super) fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) const fn params() -> FocusLoggerParams {
    FocusLoggerParams {
        provider: None,
        export_format: None,
        frequency: None,
        cron_offset_minute: None,
        interval_seconds: None,
        prefix: None,
        destination_config: None,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake exporter failed")]
pub(super) struct TestError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ExportCall {
    All {
        limit: Option<i64>,
    },
    Window {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        frequency: String,
        limit: Option<i64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedSettings {
    pub(super) provider: String,
    pub(super) export_format: String,
    pub(super) prefix: String,
    pub(super) bucket_name: Option<String>,
}

#[derive(Clone)]
pub(super) struct Recorder {
    pub(super) calls: Arc<Mutex<Vec<ExportCall>>>,
    pub(super) settings: Arc<Mutex<Vec<RecordedSettings>>>,
    pub(super) fail: bool,
    pub(super) gate: Option<Arc<Notify>>,
}

impl Recorder {
    pub(super) fn new(fail: bool) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            settings: Arc::new(Mutex::new(Vec::new())),
            fail,
            gate: None,
        }
    }

    // an export that signals the gate and never finishes, so a test can cancel the job while it waits
    pub(super) fn paused(fail: bool, gate: Arc<Notify>) -> Self {
        let mut recorder = Self::new(fail);
        recorder.gate = Some(gate);
        recorder
    }

    pub(super) fn factory(&self) -> RecordingFactory {
        RecordingFactory {
            recorder: self.clone(),
        }
    }

    pub(super) fn calls(&self) -> Vec<ExportCall> {
        self.calls.lock().unwrap().clone()
    }

    pub(super) fn settings(&self) -> Vec<RecordedSettings> {
        self.settings.lock().unwrap().clone()
    }

    fn record_settings(&self, settings: ExportSettings) {
        self.settings.lock().unwrap().push(RecordedSettings {
            provider: settings.provider,
            export_format: settings.export_format,
            prefix: settings.prefix,
            bucket_name: settings.destination_config.bucket_name,
        });
    }
}

pub(super) struct RecordingFactory {
    recorder: Recorder,
}

impl FocusEngineFactory for RecordingFactory {
    type Engine = RecordingExporter;

    fn create(&self, settings: ExportSettings) -> Result<RecordingExporter, TestError> {
        self.recorder.record_settings(settings);
        Ok(RecordingExporter {
            recorder: self.recorder.clone(),
        })
    }
}

pub(super) struct RecordingExporter {
    recorder: Recorder,
}

impl RecordingExporter {
    const fn outcome(&self) -> Result<(), TestError> {
        if self.recorder.fail {
            return Err(TestError);
        }
        Ok(())
    }

    fn record(&self, call: ExportCall) {
        self.recorder.calls.lock().unwrap().push(call);
    }

    // without a gate the export finishes at once; with one it tells the test and never finishes
    fn finish(&self) -> BoxFuture<'_, Result<(), TestError>> {
        let Some(gate) = self.recorder.gate.as_ref() else {
            return Box::pin(ready(self.outcome()));
        };
        gate.notify_one();
        Box::pin(pending())
    }
}

impl FocusExporter for RecordingExporter {
    type Error = TestError;

    fn export_all(&self, limit: Option<i64>) -> impl Future<Output = Result<(), TestError>> + Send {
        self.record(ExportCall::All { limit });
        self.finish()
    }

    fn export_window(
        &self,
        window: &FocusTimeWindow,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<(), TestError>> + Send {
        self.record(ExportCall::Window {
            start: window.start_time,
            end: window.end_time,
            frequency: window.frequency.clone(),
            limit,
        });
        self.finish()
    }
}

// A factory whose first build fails and whose second one succeeds, so a retried _ensure_engine shows.
pub(super) struct FlakyFactory {
    pub(super) recorder: Recorder,
    pub(super) attempts: AtomicUsize,
}

impl FocusEngineFactory for FlakyFactory {
    type Engine = RecordingExporter;

    fn create(&self, settings: ExportSettings) -> Result<RecordingExporter, TestError> {
        if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(TestError);
        }
        self.recorder.record_settings(settings);
        Ok(RecordingExporter {
            recorder: self.recorder.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FixedClock {
    pub(super) now: DateTime<Utc>,
}

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

// The logger most tests want: the fixed clock, the pod lock they hand in, and the factory they built.
pub(super) fn build<F>(
    environment: &BTreeMap<String, String>,
    params: FocusLoggerParams,
    factory: F,
    pod_lock: Option<Arc<dyn FocusPodLock>>,
) -> FocusLogger<F>
where
    F: FocusEngineFactory,
{
    let deps = FocusLoggerDeps {
        factory,
        clock: Arc::new(FixedClock { now: at(FIXED_NOW) }),
        pod_lock,
        tasks: TaskTracker::new(),
    };
    FocusLogger::new(environment, params, deps).expect("the logger builds")
}

// A logger whose frequency and interval come from parameters, so a test names only what it sets.
pub(super) fn configured(
    frequency: &str,
    cron_offset_minute: Option<i64>,
    interval_seconds: Option<i64>,
) -> FocusLogger<RecordingFactory> {
    let mut params = params();
    params.frequency = Some(frequency.to_owned());
    params.cron_offset_minute = cron_offset_minute;
    params.interval_seconds = interval_seconds;
    build(
        &BTreeMap::new(),
        params,
        Recorder::new(false).factory(),
        None,
    )
}

pub(super) fn overrides(bucket_name: &str) -> DestinationOverrides {
    DestinationOverrides {
        bucket_name: Some(bucket_name.to_owned()),
        region_name: None,
        endpoint_url: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        api_key: None,
        integration_token: None,
        base_url: None,
        service_account_json: None,
        api_endpoint: None,
        connection_id: None,
    }
}

pub(super) fn locked(
    recorder: Recorder,
    has_cache: bool,
    acquired: Option<bool>,
) -> (FocusLogger<RecordingFactory>, Recorder, Arc<FakePodLock>) {
    let lock = Arc::new(FakePodLock::new(has_cache, acquired));
    let logger = build(
        &BTreeMap::new(),
        params(),
        recorder.factory(),
        Some(into_pod_lock(Arc::clone(&lock))),
    );
    (logger, recorder, lock)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PodLockCall {
    Acquire(String),
    Release(String),
}

pub(super) struct FakePodLock {
    pub(super) has_cache: bool,
    pub(super) acquired: Option<bool>,
    pub(super) calls: Mutex<Vec<PodLockCall>>,
}

impl FakePodLock {
    pub(super) const fn new(has_cache: bool, acquired: Option<bool>) -> Self {
        Self {
            has_cache,
            acquired,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<PodLockCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl FocusPodLock for FakePodLock {
    fn has_cache(&self) -> bool {
        self.has_cache
    }

    fn acquire_lock<'job>(&'job self, cronjob_id: &'job str) -> BoxFuture<'job, Option<bool>> {
        self.calls
            .lock()
            .unwrap()
            .push(PodLockCall::Acquire(cronjob_id.to_owned()));
        Box::pin(ready(self.acquired))
    }

    fn release_lock<'job>(&'job self, cronjob_id: &'job str) -> BoxFuture<'job, ()> {
        self.calls
            .lock()
            .unwrap()
            .push(PodLockCall::Release(cronjob_id.to_owned()));
        Box::pin(ready(()))
    }
}

pub(super) fn into_pod_lock(lock: Arc<FakePodLock>) -> Arc<dyn FocusPodLock> {
    lock
}

// One lease per key, like the redis cache the real manager talks to, and the TTLs the manager asked for.
#[derive(Clone, Default)]
pub(super) struct SharedLockCache {
    owners: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    ttls: Arc<Mutex<Vec<i64>>>,
}

impl SharedLockCache {
    pub(super) fn ttls(&self) -> Vec<i64> {
        self.ttls.lock().unwrap().clone()
    }
}

impl LockCache for SharedLockCache {
    fn get_owner(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        let owner = self.owners.lock().unwrap().get(key).cloned();
        ready(Ok(owner))
    }

    fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        self.ttls.lock().unwrap().push(ttl_seconds);
        let mut owners = self.owners.lock().unwrap();
        let stored = if owners.contains_key(key) {
            false
        } else {
            owners.insert(key.to_owned(), owner.as_bytes().to_vec());
            true
        };
        drop(owners);
        ready(Ok(stored))
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        let removed = self.owners.lock().unwrap().remove(key);
        ready(Ok(i64::from(removed.is_some())))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

// A real pod lock manager over the shared cache, with a pod id of its own.
pub(super) fn lock_manager(
    cache: Option<Arc<SharedLockCache>>,
) -> Arc<PodLockManager<SharedLockCache>> {
    Arc::new(PodLockManager::new(
        cache,
        60,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(uuid::Uuid::new_v4()).expect("the pod id is a v4 UUID"),
    ))
}

pub(super) struct RecordingScheduler<F: FocusEngineFactory> {
    pub(super) jobs: Mutex<Vec<(Arc<FocusLogger<F>>, ScheduleTrigger)>>,
    pub(super) rejects: bool,
}

impl<F: FocusEngineFactory> RecordingScheduler<F> {
    pub(super) const fn new(rejects: bool) -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
            rejects,
        }
    }

    pub(super) fn jobs(&self) -> Vec<(Arc<FocusLogger<F>>, ScheduleTrigger)> {
        self.jobs.lock().unwrap().clone()
    }
}

impl<F: FocusEngineFactory> FocusScheduler<F> for RecordingScheduler<F> {
    type Error = TestError;

    fn add_job(
        &self,
        logger: Arc<FocusLogger<F>>,
        trigger: ScheduleTrigger,
    ) -> Result<(), TestError> {
        if self.rejects {
            return Err(TestError);
        }
        self.jobs.lock().unwrap().push((logger, trigger));
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs {
    events: Arc<Mutex<Vec<(Level, String)>>>,
}

impl Logs {
    pub(super) fn has_at(&self, level: Level, needle: &str) -> bool {
        self.events
            .lock()
            .unwrap()
            .iter()
            .any(|(event_level, message)| *event_level == level && message.contains(needle))
    }
}

struct Collector {
    logs: Logs,
}

impl<S: Subscriber> Layer<S> for Collector {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut message = String::new();
        event.record(&mut Fields {
            message: &mut message,
        });
        self.logs
            .events
            .lock()
            .unwrap()
            .push((*event.metadata().level(), message));
    }
}

struct Fields<'buffer> {
    message: &'buffer mut String,
}

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            write!(self.message, "{value:?}").expect("writing to a String cannot fail");
        }
    }
}

pub(super) fn capture_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let subscriber = registry().with(Collector { logs: logs.clone() });
    (logs, tracing::subscriber::set_default(subscriber))
}
