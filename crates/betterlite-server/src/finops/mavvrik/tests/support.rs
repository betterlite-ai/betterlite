#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, pending, ready};
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::finops::focus::engine::ExportSettings;
use betterlite_logging::finops::metering::clock::UtcClock;
use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use betterlite_logging::focus_records::transformer::UsageRow;
use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use super::error::ConfigError;
use super::export::{MavvrikEngineFactory, MavvrikExporter, MavvrikFocusLogger, MavvrikLoggerDeps};
use super::schedule::{MavvrikLazyInit, MavvrikScheduler};
use crate::finops::focus::export_schedule::{FocusPodLock, ScheduleTrigger};

pub(super) const FIXED_NOW: &str = "2024-03-10T13:47:23.123456Z";

pub(super) fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("the test timestamp is valid RFC 3339")
        .to_utc()
}

pub(super) fn day(date: &str) -> DateTime<Utc> {
    at(&format!("{date}T00:00:00Z"))
}

pub(super) fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn usage_rows(count: usize) -> Vec<UsageRow> {
    (0..count)
        .map(|index| UsageRow {
            date: String::from("2024-03-09"),
            api_key: format!("key-{index}"),
            spend: Decimal::ONE,
            api_requests: 1,
            user_id: None,
            model: None,
            model_group: None,
            custom_llm_provider: None,
            team_id: None,
            team_alias: None,
            organization_id: None,
            organization_alias: None,
            user_email: None,
            api_key_alias: None,
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
#[error("the fake Mavvrik engine failed")]
pub(super) struct TestError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EngineCall {
    Marker,
    Rows {
        limit: i64,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },
    Payload {
        rows: usize,
    },
    Filename {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        frequency: String,
    },
    Deliver {
        content: Vec<u8>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        frequency: String,
        filename: String,
    },
}

impl EngineCall {
    fn into_delivered(self) -> Option<Vec<u8>> {
        let EngineCall::Deliver { content, .. } = self else {
            return None;
        };
        Some(content)
    }

    fn into_window(self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let EngineCall::Deliver { start, end, .. } = self else {
            return None;
        };
        Some((start, end))
    }

    fn into_frequency(self) -> Option<String> {
        let EngineCall::Deliver { frequency, .. } = self else {
            return None;
        };
        Some(frequency)
    }

    fn into_limit(self) -> Option<i64> {
        let EngineCall::Rows { limit, .. } = self else {
            return None;
        };
        Some(limit)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedSettings {
    pub(super) provider: String,
    pub(super) export_format: String,
    pub(super) prefix: String,
    pub(super) bucket_name: Option<String>,
    pub(super) region_name: Option<String>,
    pub(super) endpoint_url: Option<String>,
    pub(super) aws_access_key_id: Option<String>,
    pub(super) aws_secret_access_key: Option<String>,
    pub(super) aws_session_token: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) integration_token: Option<String>,
    pub(super) base_url: Option<String>,
    pub(super) service_account_json: Option<String>,
    pub(super) api_endpoint: Option<String>,
    pub(super) connection_id: Option<String>,
}

// What the fake engine answers with and what it saw: marker, rows, payload, failure, gate, calls, settings, builds.
#[derive(Clone)]
pub(super) struct Recorder {
    pub(super) marker: MetricsMarker,
    pub(super) rows: Vec<UsageRow>,
    pub(super) payload: Option<Vec<u8>>,
    pub(super) filename: String,
    pub(super) fail: bool,
    pub(super) gate: Option<Arc<Notify>>,
    calls: Arc<Mutex<Vec<EngineCall>>>,
    settings: Arc<Mutex<Vec<RecordedSettings>>>,
    builds: Arc<AtomicUsize>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            marker: MetricsMarker::Epoch(0),
            rows: Vec::new(),
            payload: None,
            filename: String::from("mavvrik_focus_export.csv"),
            fail: false,
            gate: None,
            calls: Arc::new(Mutex::new(Vec::new())),
            settings: Arc::new(Mutex::new(Vec::new())),
            builds: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Recorder {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn factory(&self) -> FakeFactory {
        FakeFactory {
            recorder: self.clone(),
        }
    }

    pub(super) fn calls(&self) -> Vec<EngineCall> {
        self.calls.lock().unwrap().clone()
    }

    pub(super) fn settings(&self) -> Vec<RecordedSettings> {
        self.settings.lock().unwrap().clone()
    }

    pub(super) fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }

    fn collected<T, P>(&self, pick: P) -> Vec<T>
    where
        P: Fn(EngineCall) -> Option<T>,
    {
        self.calls().into_iter().filter_map(pick).collect()
    }

    pub(super) fn delivered(&self) -> Vec<Vec<u8>> {
        self.collected(EngineCall::into_delivered)
    }

    pub(super) fn windows(&self) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
        self.collected(EngineCall::into_window)
    }

    pub(super) fn frequencies(&self) -> Vec<String> {
        self.collected(EngineCall::into_frequency)
    }

    pub(super) fn limits(&self) -> Vec<i64> {
        self.collected(EngineCall::into_limit)
    }

    pub(super) fn record(&self, call: EngineCall) {
        self.calls.lock().unwrap().push(call);
    }

    pub(super) fn record_settings(&self, settings: ExportSettings) {
        let config = settings.destination_config;
        self.settings.lock().unwrap().push(RecordedSettings {
            provider: settings.provider,
            export_format: settings.export_format,
            prefix: settings.prefix,
            bucket_name: config.bucket_name,
            region_name: config.region_name,
            endpoint_url: config.endpoint_url,
            aws_access_key_id: config
                .aws_access_key_id
                .map(|value| value.expose_secret().to_owned()),
            aws_secret_access_key: config
                .aws_secret_access_key
                .map(|value| value.expose_secret().to_owned()),
            aws_session_token: config
                .aws_session_token
                .map(|value| value.expose_secret().to_owned()),
            api_key: config.api_key.map(|value| value.expose_secret().to_owned()),
            integration_token: config
                .integration_token
                .map(|value| value.expose_secret().to_owned()),
            base_url: config.base_url,
            service_account_json: config
                .service_account_json
                .map(|value| value.expose_secret().to_owned()),
            api_endpoint: config.api_endpoint,
            connection_id: config.connection_id,
        });
    }
}

pub(super) struct FakeFactory {
    recorder: Recorder,
}

pub(super) struct FakeEngine {
    recorder: Recorder,
}

impl MavvrikEngineFactory for FakeFactory {
    type Engine = FakeEngine;

    fn create(&self, settings: ExportSettings) -> Result<FakeEngine, TestError> {
        self.recorder.builds.fetch_add(1, Ordering::SeqCst);
        self.recorder.record_settings(settings);
        Ok(FakeEngine {
            recorder: self.recorder.clone(),
        })
    }
}

impl MavvrikExporter for FakeEngine {
    type Error = TestError;

    fn metrics_marker(&self) -> impl Future<Output = Result<MetricsMarker, TestError>> + Send {
        self.recorder.record(EngineCall::Marker);
        let gate = self.recorder.gate.clone();
        let fail = self.recorder.fail;
        let marker = self.recorder.marker.clone();
        async move {
            if let Some(gate) = gate {
                gate.notify_one();
                pending::<()>().await;
            }
            if fail {
                return Err(TestError);
            }
            Ok(marker)
        }
    }

    fn usage_rows(
        &self,
        limit: i64,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<UsageRow>, TestError>> + Send {
        self.recorder.record(EngineCall::Rows { limit, start, end });
        let rows = self.recorder.rows.clone();
        ready(Ok::<Vec<UsageRow>, TestError>(rows))
    }

    fn payload(&self, rows: &[UsageRow]) -> Result<Vec<u8>, TestError> {
        self.recorder
            .record(EngineCall::Payload { rows: rows.len() });
        if let Some(payload) = &self.recorder.payload {
            return Ok(payload.clone());
        }
        Ok(format!("rows:{}", rows.len()).into_bytes())
    }

    fn filename(&self, window: &FocusTimeWindow) -> Result<String, TestError> {
        self.recorder.record(EngineCall::Filename {
            start: window.start_time,
            end: window.end_time,
            frequency: window.frequency.clone(),
        });
        Ok(self.recorder.filename.clone())
    }

    fn deliver(
        &self,
        content: &[u8],
        window: &FocusTimeWindow,
        filename: &str,
    ) -> impl Future<Output = Result<(), TestError>> + Send {
        self.recorder.record(EngineCall::Deliver {
            content: content.to_vec(),
            start: window.start_time,
            end: window.end_time,
            frequency: window.frequency.clone(),
            filename: filename.to_owned(),
        });
        ready(Ok::<(), TestError>(()))
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

pub(super) fn deps(pod_lock: Option<Arc<dyn FocusPodLock>>) -> MavvrikLoggerDeps {
    MavvrikLoggerDeps {
        clock: Arc::new(FixedClock { now: at(FIXED_NOW) }),
        pod_lock,
        tasks: TaskTracker::new(),
    }
}

pub(super) fn try_build(
    environment: &BTreeMap<String, String>,
    recorder: &Recorder,
    pod_lock: Option<Arc<dyn FocusPodLock>>,
) -> Result<MavvrikFocusLogger<FakeFactory>, ConfigError> {
    MavvrikFocusLogger::new(environment, recorder.factory(), deps(pod_lock))
}

pub(super) fn build(
    environment: &BTreeMap<String, String>,
    recorder: &Recorder,
    pod_lock: Option<Arc<dyn FocusPodLock>>,
) -> MavvrikFocusLogger<FakeFactory> {
    try_build(environment, recorder, pod_lock).expect("the logger builds")
}

pub(super) fn pod_lock(
    has_cache: bool,
    acquired: Option<bool>,
) -> (Arc<FakePodLock>, Arc<dyn FocusPodLock>) {
    let lock = Arc::new(FakePodLock::new(has_cache, acquired));
    let port: Arc<dyn FocusPodLock> = Arc::<FakePodLock>::clone(&lock);
    (lock, port)
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

// a recorded add_job call: the logger, its job id, the trigger and replace_existing, in call order
pub(super) type Job<F> = (Arc<MavvrikFocusLogger<F>>, String, ScheduleTrigger, bool);

pub(super) struct RecordingScheduler<F: MavvrikEngineFactory> {
    jobs: Mutex<Vec<Job<F>>>,
    rejects: bool,
}

impl<F: MavvrikEngineFactory> RecordingScheduler<F> {
    pub(super) const fn new(rejects: bool) -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
            rejects,
        }
    }

    pub(super) fn jobs(&self) -> Vec<Job<F>> {
        self.jobs.lock().unwrap().clone()
    }
}

impl<F: MavvrikEngineFactory> MavvrikScheduler<F> for RecordingScheduler<F> {
    type Error = TestError;

    fn add_job(
        &self,
        logger: Arc<MavvrikFocusLogger<F>>,
        job_id: &str,
        trigger: ScheduleTrigger,
        replace_existing: bool,
    ) -> Result<(), TestError> {
        if self.rejects {
            return Err(TestError);
        }
        self.jobs.lock().map_err(|_error| TestError)?.push((
            logger,
            job_id.to_owned(),
            trigger,
            replace_existing,
        ));
        Ok(())
    }
}

pub(super) struct FakeLazyInit<F: MavvrikEngineFactory> {
    instance: Option<Arc<MavvrikFocusLogger<F>>>,
    integrations: Mutex<Vec<String>>,
}

impl<F: MavvrikEngineFactory> FakeLazyInit<F> {
    pub(super) const fn new(instance: Option<Arc<MavvrikFocusLogger<F>>>) -> Self {
        Self {
            instance,
            integrations: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn integrations(&self) -> Vec<String> {
        self.integrations.lock().unwrap().clone()
    }
}

impl<F: MavvrikEngineFactory> MavvrikLazyInit<F> for FakeLazyInit<F> {
    fn initialize(&self, logging_integration: &str) -> Option<Arc<MavvrikFocusLogger<F>>> {
        self.integrations
            .lock()
            .unwrap()
            .push(logging_integration.to_owned());
        self.instance.clone()
    }
}

// The log lines a test's subscriber wrote, searched by level and by the part of the message it cares about.
#[derive(Clone, Default)]
pub(super) struct Logs {
    bytes: Arc<Mutex<Vec<u8>>>,
}

pub(super) struct Sink {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Logs {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .bytes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the captured log output is UTF-8")
    }

    pub(super) fn has_at(&self, level: Level, needle: &str) -> bool {
        self.text()
            .lines()
            .any(|line| line.contains(level.as_str()) && line.contains(needle))
    }
}

pub(super) fn capture_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let sink = Sink {
        bytes: Arc::clone(&logs.bytes),
    };
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || Sink {
            bytes: Arc::clone(&sink.bytes),
        })
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .finish();
    (logs, set_default(subscriber))
}
