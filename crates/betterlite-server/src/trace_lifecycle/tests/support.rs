use std::fs::read_to_string;
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::clickhouse::clickhouse_logging::context::LensAnalysis;
use betterlite_logging::clickhouse::clickhouse_logging::settings::ClickHouseSettings;
use betterlite_logging::clickhouse::clickhouse_logging::spend_logger::ClickHouseSpendLogger;
use betterlite_logging::clickhouse::clickhouse_logging::storage::ClickHouseStorage;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::Registration;
use betterlite_logging::registry::identity::CallbackIdentity;
use betterlite_logging::registry::phases::CallbackList;
use betterlite_logging::registry::sink::CallbackSink;
use betterlite_logging::trace_types::SpendLogRecord;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::trace_lifecycle::error::{ReceiverFailure, StartupFailure};
use crate::trace_lifecycle::ports::{ReceiverFactory, ReceiverHandle};
use crate::trace_lifecycle::spend_logger::{
    ClickHouseSpendLoggers, SharedStorage, SpendLoggerFactory,
};

const PAYLOAD_TEMPLATE: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "@call_type",
    "response_cost": 0.5,
    "status": "@status",
    "status_fields": {"llm_api_status": "@status", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.0,
    "completionStartTime": 1704067201.0,
    "response_time": 1.0,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "the secret prompt"}],
    "response": null,
    "error_str": "provider exploded"
}"#;

#[derive(Clone)]
pub(crate) struct Insertion {
    pub(crate) table: String,
    pub(crate) rows: Vec<SpendLogRecord>,
}

#[derive(Clone, Default)]
pub(crate) struct RecordingStorage {
    state: Arc<RecordingState>,
}

#[derive(Default)]
struct RecordingState {
    insertions: Mutex<Vec<Insertion>>,
    attempts: AtomicUsize,
    remaining_failures: AtomicUsize,
    probe: Mutex<Option<Weak<dyn CallbackLogger>>>,
    probe_counts: Mutex<Vec<usize>>,
}

impl RecordingStorage {
    pub(crate) fn fail_inserts(&self, count: usize) {
        self.state.remaining_failures.store(count, Ordering::SeqCst);
    }

    pub(crate) fn watch_strong_count(&self, logger: &Arc<dyn CallbackLogger>) {
        let probe = Arc::downgrade(logger);
        *self
            .state
            .probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(probe);
    }

    pub(crate) fn insertions(&self) -> Vec<Insertion> {
        self.state
            .insertions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn attempts(&self) -> usize {
        self.state.attempts.load(Ordering::SeqCst)
    }

    pub(crate) fn probe_counts(&self) -> Vec<usize> {
        self.state
            .probe_counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClickHouseStorage for RecordingStorage {
    type Error = io::Error;

    fn insert_rows(
        &self,
        table: &str,
        rows: &[SpendLogRecord],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.state.attempts.fetch_add(1, Ordering::SeqCst);
        let observed = self
            .state
            .probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, Weak::strong_count);
        self.state
            .probe_counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(observed);
        let remaining = self.state.remaining_failures.load(Ordering::SeqCst);
        if remaining > 0 {
            self.state
                .remaining_failures
                .store(remaining.saturating_sub(1), Ordering::SeqCst);
            return ready(Err(io::Error::other(
                "the recording storage rejected the batch",
            )));
        }
        self.state
            .insertions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Insertion {
                table: table.to_owned(),
                rows: rows.to_vec(),
            });
        ready(Ok(()))
    }

    fn ensure_schema(
        &self,
        _trace_retention_days: u32,
        _spend_log_retention_days: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        ready(Ok(()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildBehavior {
    Built,
    MissingSetting,
    Unexpected,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartBehavior {
    Started,
    Io,
    Runtime,
    InvalidValue,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FakeReceiverError {
    #[error("missing setting {setting}")]
    MissingSetting { setting: String },
    #[error("agent tracing storage failed")]
    Storage {
        #[source]
        source: io::Error,
    },
    #[error("the ingest semaphore is closed")]
    Runtime,
    #[error("the concurrency limit is negative")]
    InvalidValue,
    #[error("the receiver pool vanished")]
    Unexpected,
}

impl ReceiverFailure for FakeReceiverError {
    fn startup_failure(&self) -> StartupFailure {
        match self {
            Self::MissingSetting { .. } => StartupFailure::MissingSetting,
            Self::Storage { .. } => StartupFailure::Storage,
            Self::Runtime => StartupFailure::Runtime,
            Self::InvalidValue => StartupFailure::InvalidValue,
            Self::Unexpected => StartupFailure::Unexpected,
        }
    }
}

pub(crate) struct FakeFactory {
    storage: Arc<RecordingStorage>,
    build_behavior: BuildBehavior,
    start_behavior: StartBehavior,
    builds: Arc<AtomicUsize>,
    starts: Arc<AtomicUsize>,
}

impl FakeFactory {
    pub(crate) fn new(storage: Arc<RecordingStorage>) -> Self {
        Self {
            storage,
            build_behavior: BuildBehavior::Built,
            start_behavior: StartBehavior::Started,
            builds: Arc::new(AtomicUsize::new(0)),
            starts: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn with_build(mut self, behavior: BuildBehavior) -> Self {
        self.build_behavior = behavior;
        self
    }

    pub(crate) fn with_start(mut self, behavior: StartBehavior) -> Self {
        self.start_behavior = behavior;
        self
    }

    pub(crate) fn build_count(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }

    pub(crate) fn start_count(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
}

impl ReceiverFactory for FakeFactory {
    type Receiver = FakeReceiver;
    type Error = FakeReceiverError;

    fn build(&self) -> Result<FakeReceiver, FakeReceiverError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        match self.build_behavior {
            BuildBehavior::Built => Ok(FakeReceiver {
                storage: Arc::clone(&self.storage),
                start_behavior: self.start_behavior,
                starts: Arc::clone(&self.starts),
            }),
            BuildBehavior::MissingSetting => Err(FakeReceiverError::MissingSetting {
                setting: "CLICKHOUSE_URL".to_owned(),
            }),
            BuildBehavior::Unexpected => Err(FakeReceiverError::Unexpected),
        }
    }
}

pub(crate) struct FakeReceiver {
    pub(crate) storage: Arc<RecordingStorage>,
    start_behavior: StartBehavior,
    starts: Arc<AtomicUsize>,
}

impl ReceiverHandle for FakeReceiver {
    type Storage = RecordingStorage;
    type Error = FakeReceiverError;

    fn storage(&self) -> Arc<RecordingStorage> {
        Arc::clone(&self.storage)
    }

    fn start(&self) -> impl Future<Output = Result<(), FakeReceiverError>> + Send {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let behavior = self.start_behavior;
        async move {
            match behavior {
                StartBehavior::Started => Ok(()),
                StartBehavior::Io => Err(FakeReceiverError::Storage {
                    source: io::Error::other("schema setup failed"),
                }),
                StartBehavior::Runtime => Err(FakeReceiverError::Runtime),
                StartBehavior::InvalidValue => Err(FakeReceiverError::InvalidValue),
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the spend logger could not be built")]
pub(crate) struct FailingLoggerError;

pub(crate) struct FailingLoggers;

impl<Storage: ClickHouseStorage> SpendLoggerFactory<Storage> for FailingLoggers {
    type Error = FailingLoggerError;

    fn build(
        &self,
        _storage: Arc<Storage>,
    ) -> Result<ClickHouseSpendLogger<SharedStorage<Storage>>, FailingLoggerError> {
        Err(FailingLoggerError)
    }

    fn lens(&self) -> LensAnalysis {
        LensAnalysis::default()
    }
}

pub(crate) struct OperatorLogger;

impl CallbackLogger for OperatorLogger {
    fn callback_name(&self) -> &'static str {
        "clickhouse"
    }
}

pub(crate) fn register_operator(registry: &mut CallbackRegistry, lists: &[CallbackList]) {
    let operator: Arc<dyn CallbackLogger> = Arc::new(OperatorLogger);
    let identity = CallbackIdentity {
        class_name: "clickhouse".to_owned(),
        attributes: IndexMap::new(),
    };
    for list in lists {
        let registration = registry.register(
            identity.clone(),
            CallbackSink::Logger(Arc::clone(&operator)),
            *list,
        );
        assert_eq!(registration, Registration::Registered);
    }
}

pub(crate) fn assert_only_identity(
    registry: &CallbackRegistry,
    lists: &[CallbackList],
    class_name: &str,
) {
    for list in lists {
        let identities = registry.list_identities(*list);
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].class_name, class_name);
    }
}

pub(crate) fn spend_settings(batch_size: usize, max_retries: usize) -> ClickHouseSettings {
    ClickHouseSettings {
        database: "litellm".to_owned(),
        url: SecretString::from("http://clickhouse:8123".to_owned()),
        batch_size,
        flush_interval: Duration::from_secs(3600),
        max_buffered_rows: 1000,
        max_retries,
    }
}

pub(crate) fn spend_loggers(settings: ClickHouseSettings) -> ClickHouseSpendLoggers {
    ClickHouseSpendLoggers {
        settings,
        tasks: TaskTracker::new(),
        lens: LensAnalysis::default(),
    }
}

fn payload(call_type: &str, status: &str) -> StandardLoggingPayload {
    let source = PAYLOAD_TEMPLATE
        .replace("@call_type", call_type)
        .replace("@status", status);
    serde_json::from_str(&source).expect("the payload fixture parses")
}

pub(crate) fn event(call_type: &str, status: &str) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload(call_type, status)),
        dynamic_params: Arc::new(Json::Null),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(crate) fn event_with_session(
    session_id: &str,
    trace_id: &str,
    metadata_session_id: &str,
) -> Arc<LoggerEvent> {
    let mut built = payload("completion", "success");
    built.session_id = session_id.to_owned();
    built.trace_id = trace_id.to_owned();
    built.metadata.requester_metadata = Some(Json::Object(IndexMap::from([(
        "session_id".to_owned(),
        Json::Text(metadata_session_id.to_owned()),
    )])));
    Arc::new(LoggerEvent {
        payload: Arc::new(built),
        dynamic_params: Arc::new(Json::Null),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(crate) fn setup() -> (FakeFactory, Arc<RecordingStorage>, CallbackRegistry) {
    let storage = Arc::new(RecordingStorage::default());
    let factory = FakeFactory::new(Arc::clone(&storage));
    (factory, storage, CallbackRegistry::new(100))
}

pub(crate) fn capture_log() -> (tempfile::NamedTempFile, DefaultGuard) {
    let log = tempfile::NamedTempFile::new().expect("the log file opens");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("the log reopens")))
        .finish();
    (log, set_default(subscriber))
}

pub(crate) fn log_text(log: &tempfile::NamedTempFile) -> String {
    read_to_string(log.path()).expect("the log reads")
}
