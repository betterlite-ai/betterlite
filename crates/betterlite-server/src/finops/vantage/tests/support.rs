#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, pending, ready};
use std::sync::{Arc, Mutex};

use betterlite_logging::finops::focus::engine::ExportSettings;
use betterlite_logging::finops::metering::clock::UtcClock;
use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use betterlite_logging::registry::identity::{Attribute, CallbackIdentity};
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::finops::focus::error::ScheduleError;
use crate::finops::focus::export::{FocusEngineFactory, FocusExporter, FocusLoggerDeps};
use crate::finops::focus::export_schedule::{FocusPodLock, ScheduleTrigger};
use crate::finops::vantage::error::VantageExportError;
use crate::finops::vantage::export::{VantageExportLogger, VantageLogger};
use crate::finops::vantage::export_schedule::VantageScheduler;
use crate::finops::vantage::settings::VantageLoggerParams;

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

pub(super) const fn params() -> VantageLoggerParams {
    VantageLoggerParams {
        api_key: None,
        integration_token: None,
        base_url: None,
        frequency: None,
        interval_seconds: None,
        cron_offset_minute: None,
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

#[derive(Clone)]
pub(super) struct Recorder {
    pub(super) calls: Arc<Mutex<Vec<ExportCall>>>,
    pub(super) fail: bool,
    pub(super) gate: Option<Arc<Notify>>,
}

impl Recorder {
    pub(super) fn new(fail: bool) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            fail,
            gate: None,
        }
    }

    // an export that signals the gate and never finishes, so a test can cancel the job while it waits
    pub(super) fn paused(gate: Arc<Notify>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            fail: false,
            gate: Some(gate),
        }
    }

    pub(super) fn factory(&self) -> RecordingFactory {
        RecordingFactory {
            recorder: self.clone(),
        }
    }

    pub(super) fn calls(&self) -> Vec<ExportCall> {
        self.calls.lock().unwrap().clone()
    }
}

pub(super) struct RecordingFactory {
    recorder: Recorder,
}

impl FocusEngineFactory for RecordingFactory {
    type Engine = RecordingExporter;

    fn create(&self, _settings: ExportSettings) -> Result<RecordingExporter, TestError> {
        Ok(RecordingExporter {
            recorder: self.recorder.clone(),
        })
    }
}

pub(super) struct RecordingExporter {
    recorder: Recorder,
}

impl RecordingExporter {
    fn record(&self, call: ExportCall) {
        self.recorder.calls.lock().unwrap().push(call);
    }

    // without a gate the export finishes at once; with one it tells the test and never finishes
    fn finish(&self) -> BoxFuture<'_, Result<(), TestError>> {
        let Some(gate) = self.recorder.gate.as_ref() else {
            let outcome = if self.recorder.fail {
                Err(TestError)
            } else {
                Ok(())
            };
            return Box::pin(ready(outcome));
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

#[derive(Clone, Copy, Debug)]
pub(super) struct FixedClock {
    pub(super) now: DateTime<Utc>,
}

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

pub(super) fn build(
    environment: &BTreeMap<String, String>,
    params: VantageLoggerParams,
    recorder: &Recorder,
    pod_lock: Option<Arc<dyn FocusPodLock>>,
) -> VantageLogger<RecordingFactory> {
    let deps = FocusLoggerDeps {
        factory: recorder.factory(),
        clock: Arc::new(FixedClock { now: at(FIXED_NOW) }),
        pod_lock,
        tasks: TaskTracker::new(),
    };
    VantageLogger::new(environment, params, deps).expect("the logger builds")
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

pub(super) fn locked(
    recorder: &Recorder,
    has_cache: bool,
    acquired: Option<bool>,
) -> (VantageLogger<RecordingFactory>, Arc<FakePodLock>) {
    let lock = Arc::new(FakePodLock::new(has_cache, acquired));
    let pod_lock: Arc<dyn FocusPodLock> = Arc::<FakePodLock>::clone(&lock);
    let logger = build(&BTreeMap::new(), params(), recorder, Some(pod_lock));
    (logger, lock)
}

// A registered instance that is not VantageLogger itself, standing in for a subclass of the source's class.
pub(super) struct FakeVantageLogger {
    pub(super) trigger: ScheduleTrigger,
}

impl VantageExportLogger for FakeVantageLogger {
    fn registration_identity(&self) -> CallbackIdentity {
        CallbackIdentity {
            class_name: "FakeVantageLogger".to_owned(),
            attributes: IndexMap::from([(
                "trigger".to_owned(),
                Attribute::Text(format!("{:?}", self.trigger)),
            )]),
        }
    }

    fn scheduler_trigger(&self) -> Result<ScheduleTrigger, ScheduleError> {
        Ok(self.trigger)
    }

    fn initialize_focus_export_job(&self) -> BoxFuture<'_, Result<(), VantageExportError>> {
        Box::pin(ready(Ok(())))
    }

    fn export_usage_data(
        &self,
        _limit: Option<i64>,
        _start_time_utc: Option<DateTime<Utc>>,
        _end_time_utc: Option<DateTime<Utc>>,
    ) -> BoxFuture<'_, Result<(), VantageExportError>> {
        Box::pin(ready(Ok(())))
    }
}

pub(super) struct RecordingScheduler {
    pub(super) jobs: Mutex<Vec<(Arc<dyn VantageExportLogger>, ScheduleTrigger)>>,
    pub(super) rejects: bool,
}

impl RecordingScheduler {
    pub(super) const fn new(rejects: bool) -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
            rejects,
        }
    }

    pub(super) fn jobs(&self) -> Vec<(Arc<dyn VantageExportLogger>, ScheduleTrigger)> {
        self.jobs.lock().unwrap().clone()
    }
}

impl VantageScheduler for RecordingScheduler {
    type Error = TestError;

    fn add_job(
        &self,
        logger: Arc<dyn VantageExportLogger>,
        trigger: ScheduleTrigger,
    ) -> Result<(), TestError> {
        if self.rejects {
            return Err(TestError);
        }
        self.jobs
            .lock()
            .map_err(|_error| TestError)?
            .push((logger, trigger));
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

struct Sink {
    logs: Logs,
}

impl<S: Subscriber> Layer<S> for Sink {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut message = String::new();
        event.record(&mut MessageText {
            message: &mut message,
        });
        self.logs
            .events
            .lock()
            .unwrap()
            .push((*event.metadata().level(), message));
    }
}

struct MessageText<'buffer> {
    message: &'buffer mut String,
}

impl Visit for MessageText<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            let rendered = format!("{value:?}");
            self.message.push_str(&rendered);
        }
    }
}

pub(super) fn capture_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let subscriber = registry().with(Sink { logs: logs.clone() });
    (logs, set_default(subscriber))
}
