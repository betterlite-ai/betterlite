#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::audit::Settings as AuditSettings;
use betterlite_config::sections::audit_events::Settings as SectionSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::audit::callbacks::{
    AuditLoggerBuildError, DedicatedAuditLoggerBuilder, NamedAuditCallbacks,
};
use betterlite_logging::audit::delivery::AuditDelivery;
use betterlite_logging::audit::payload::StandardAuditLogPayload;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::registry::error::RegistryError;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use tempfile::TempDir;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::audit::callbacks::AuditCallbackEntry;
use crate::audit::service::{AuditService, AuditServiceDeps};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::{AuditClock, AuditLogRepository, AuditRepositoryError};

pub(super) const NOW: &str = "2024-05-06T07:08:09.123+00:00";

pub(super) struct FixedClock {
    pub(super) now: DateTime<FixedOffset>,
}

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

pub(super) fn clock() -> Arc<FixedClock> {
    Arc::new(FixedClock {
        now: DateTime::parse_from_rfc3339(NOW).expect("the fixed clock is a timestamp"),
    })
}

pub(super) fn record() -> AuditLogRecord {
    AuditLogRecord {
        id: "audit-1".to_owned(),
        updated_at: DateTime::parse_from_rfc3339(NOW).expect("the record time is a timestamp"),
        changed_by: Some("admin".to_owned()),
        changed_by_api_key: Some(SecretString::from("sk-key".to_owned())),
        action: AuditAction::Updated,
        table_name: AuditTable::Team,
        object_id: "team-1".to_owned(),
        before_value: Some(serde_json::from_str(r#"{"api_key":"sk-1*****7890"}"#).unwrap()),
        updated_values: None,
    }
}

pub(super) fn enabled() -> AuditSettings {
    AuditSettings {
        config: SectionSettings {
            store_audit_logs: Some(true),
            store_audit_logs_env: None,
            audit_log_callbacks: vec!["otel".to_owned()],
            s3_audit_callback_params: None,
        },
        audit_log_callbacks_present: true,
    }
}

pub(super) fn disabled() -> AuditSettings {
    let mut settings = enabled();
    settings.config.store_audit_logs = Some(false);
    settings
}

pub(super) fn worker_settings(queue: usize) -> WorkerSettings {
    WorkerSettings {
        max_queue_size: queue,
        concurrency: 1,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(5),
        timeout_summary_window: Duration::from_secs(1),
    }
}

pub(super) struct RecordingLogger {
    pub(super) name: String,
    pub(super) fails: bool,
    pub(super) delivered: Mutex<Vec<StandardAuditLogPayload>>,
    pub(super) reached: Arc<AtomicBool>,
    pub(super) order: Option<Arc<Mutex<Vec<String>>>>,
}

impl RecordingLogger {
    pub(super) fn new(name: &str, fails: bool) -> Self {
        Self {
            name: name.to_owned(),
            fails,
            delivered: Mutex::new(Vec::new()),
            reached: Arc::new(AtomicBool::new(false)),
            order: None,
        }
    }

    pub(super) fn watching(name: &str, order: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            order: Some(order),
            ..Self::new(name, false)
        }
    }
}

#[async_trait]
impl CallbackLogger for RecordingLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    async fn async_log_audit_log_event(
        &self,
        audit_log: &StandardAuditLogPayload,
    ) -> Result<(), LoggerError> {
        self.reached.store(true, Ordering::SeqCst);
        if let Some(order) = &self.order {
            order
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("callback:{}", self.name));
        }
        if self.fails {
            return Err(LoggerError::Delivery {
                callback: self.name.clone(),
                message: "the audit log callback failed".to_owned(),
            });
        }
        self.delivered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(audit_log.clone());
        Ok(())
    }
}

pub(super) fn logger_entry(name: &str, fails: bool) -> (AuditCallbackEntry, Arc<RecordingLogger>) {
    let recorder = Arc::new(RecordingLogger::new(name, fails));
    let logger: Arc<dyn CallbackLogger> = Arc::clone(&recorder);
    (AuditCallbackEntry::Logger(logger), recorder)
}

#[derive(Default)]
pub(super) struct MapResolver {
    pub(super) loggers: Mutex<BTreeMap<String, Arc<dyn CallbackLogger>>>,
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) refuse: bool,
}

impl MapResolver {
    pub(super) fn with(names: Vec<(&str, Arc<dyn CallbackLogger>)>) -> Self {
        Self {
            loggers: Mutex::new(
                names
                    .into_iter()
                    .map(|(name, logger)| (name.to_owned(), logger))
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
            refuse: false,
        }
    }
}

impl LoggerResolver for MapResolver {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
        if self.refuse {
            return Err(RegistryError::UnavailableAdapter {
                name: name.to_owned(),
            });
        }
        Ok(self
            .loggers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned())
    }
}

#[derive(Default)]
pub(super) struct MapBuilder {
    pub(super) logger: Option<Arc<dyn CallbackLogger>>,
    pub(super) calls: Mutex<Vec<JsonValue>>,
}

impl DedicatedAuditLoggerBuilder for MapBuilder {
    fn build(&self, params: &JsonValue) -> Result<Arc<dyn CallbackLogger>, AuditLoggerBuildError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(params.clone());
        match &self.logger {
            Some(logger) => Ok(Arc::clone(logger)),
            None => Err(AuditLoggerBuildError::Parameters {
                source: serde_json::from_str::<JsonValue>("{").expect_err("the text is not JSON"),
            }),
        }
    }
}

pub(super) struct RecordingRepository {
    pub(super) available: bool,
    pub(super) fails: bool,
    pub(super) require_callback_first: bool,
    pub(super) inserted: Mutex<Vec<AuditLogRecord>>,
    pub(super) callback_reached: Arc<AtomicBool>,
    pub(super) order: Arc<Mutex<Vec<String>>>,
}

impl RecordingRepository {
    pub(super) fn new(available: bool, fails: bool) -> Self {
        Self {
            available,
            fails,
            require_callback_first: false,
            inserted: Mutex::new(Vec::new()),
            callback_reached: Arc::new(AtomicBool::new(false)),
            order: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl AuditLogRepository for RecordingRepository {
    fn available(&self) -> bool {
        self.available
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        if self.require_callback_first {
            // the callback worker runs on the same thread, so give it exactly as many turns as it needs to start
            for _ in 0..64 {
                if self.callback_reached.load(Ordering::SeqCst) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }
        let reached = self.callback_reached.load(Ordering::SeqCst);
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("insert:{reached}"));
        if self.fails {
            return Err(AuditRepositoryError::Unavailable);
        }
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

pub(super) struct PanickingRepository;

#[async_trait]
impl AuditLogRepository for PanickingRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, _record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        panic!("the audit write panicked")
    }
}

pub(super) struct Fixture {
    pub(super) settings: AuditSettings,
    pub(super) callbacks: Vec<AuditCallbackEntry>,
    pub(super) resolver: Arc<dyn LoggerResolver>,
    pub(super) dedicated: Arc<dyn DedicatedAuditLoggerBuilder>,
    pub(super) repository: Arc<dyn AuditLogRepository>,
    pub(super) queue: usize,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            settings: enabled(),
            callbacks: Vec::new(),
            resolver: Arc::new(MapResolver::default()),
            dedicated: Arc::new(MapBuilder::default()),
            repository: Arc::new(RecordingRepository::new(true, false)),
            queue: 8,
        }
    }

    pub(super) fn start(self) -> AuditService {
        let queue = self.queue;
        let delivery = Arc::new(AuditDelivery {
            named: NamedAuditCallbacks {
                resolver: self.resolver,
                dedicated: self.dedicated,
                cache: Mutex::new(BTreeMap::new()),
            },
            worker: LoggingWorker::start(worker_settings(queue)).expect("the delivery starts"),
        });
        AuditService::start(AuditServiceDeps {
            settings: self.settings,
            callbacks: self.callbacks,
            delivery,
            repository: self.repository,
            clock: clock(),
            new_id: Arc::new(|| "audit-id-1".to_owned()),
            worker_settings: worker_settings(queue),
        })
        .expect("the audit service starts")
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

pub(super) struct Workspace {
    pub(super) directory: TempDir,
    pub(super) config_path: PathBuf,
}

impl Workspace {
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().expect("the workspace exists");
        let config_path = directory.path().join("config.yaml");
        std::fs::write(&config_path, b"model_list: []\n").expect("the config is written");
        Self {
            directory,
            config_path,
        }
    }

    pub(super) fn file(&self, relative: &str, source: &str) {
        let path = self.directory.path().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the package directory is created");
        }
        std::fs::write(path, source).expect("the fixture module is written");
    }

    pub(super) fn module(&self, name: &str, source: &str) -> String {
        self.file(&format!("{name}.py"), source);
        name.to_owned()
    }
}
