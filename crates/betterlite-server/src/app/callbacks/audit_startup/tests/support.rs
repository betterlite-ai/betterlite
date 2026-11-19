#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::audit::Settings as AuditSettings;
use betterlite_config::audit::load_audit_settings;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::audit_events::Settings as AuditPolicy;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::audit::callbacks::{
    AuditLoggerBuildError, DedicatedAuditLoggerBuilder, NamedAuditCallbacks,
};
use betterlite_logging::audit::delivery::{AuditCallback, AuditDelivery};
use betterlite_logging::audit::payload::StandardAuditLogPayload;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::registry::catalog::CatalogError;
use betterlite_logging::registry::error::RegistryError;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use tempfile::TempDir;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::app::callbacks::audit_startup::{AuditStartupError, install_audit_callbacks};
use crate::audit::callbacks::AuditCallbackEntry;
use crate::audit::service::{AuditService, AuditServiceDeps};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::{AuditClock, AuditLogRepository, AuditRepositoryError};
use crate::python_hooks::PythonHooks;

const NOW: &str = "2024-05-06T07:08:09.123+00:00";

const CUSTOM_LOGGER: &str = "
class CustomLogger:
    async def async_log_audit_log_event(self, audit_log):
        pass
";

const RECORDER_MODULE: &str = r#"
from betterlite.integrations.custom_logger import CustomLogger

class Recorder(CustomLogger):
    async def async_log_audit_log_event(self, audit_log):
        with open(r"__OUTPUT__", "a", encoding="utf-8") as handle:
            handle.write(audit_log["id"] + "\n")

recorder = Recorder()
"#;

pub(super) const BROKEN_MODULE: &str =
    "raise RuntimeError(\"the audit callback module cannot import\")\n";

pub(super) struct LoadedConfig {
    pub(super) settings: AuditSettings,
    pub(super) policy: AuditPolicy,
    pub(super) configured: Option<Vec<String>>,
}

pub(super) fn load(source: &str, environment: &BTreeMap<String, String>) -> LoadedConfig {
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .expect("the config loads");
    let settings = load_audit_settings(source, environment).expect("the audit settings load");
    let configured = settings
        .audit_log_callbacks_present
        .then_some(settings.config.audit_log_callbacks.clone());
    LoadedConfig {
        settings,
        policy: config.sections.audit_events,
        configured,
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) struct Workspace {
    pub(super) directory: TempDir,
    pub(super) config_path: PathBuf,
}

impl Workspace {
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().expect("the workspace exists");
        let config_path = directory.path().join("config.yaml");
        fs::write(&config_path, b"model_list: []\n").expect("the config is written");
        let workspace = Self {
            directory,
            config_path,
        };
        workspace.file("betterlite/__init__.py", "");
        workspace.file("betterlite/integrations/__init__.py", "");
        workspace.file("betterlite/integrations/custom_logger.py", CUSTOM_LOGGER);
        workspace
    }

    pub(super) fn file(&self, relative: &str, source: &str) {
        let path = self.directory.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("the package directory is created");
        }
        fs::write(path, source).expect("the fixture module is written");
    }

    pub(super) fn module(&self, name: &str, source: &str) -> String {
        self.file(&format!("{name}.py"), source);
        name.to_owned()
    }

    pub(super) fn recorder(&self) -> String {
        let output = self.directory.path().join("delivered.txt");
        let path = output.to_str().expect("the fixture path is UTF-8");
        let source = RECORDER_MODULE.replace("__OUTPUT__", path);
        self.module("b8872_recorder", &source)
    }
}

pub(super) struct RecordingLogger {
    pub(super) name: String,
    pub(super) delivered: Mutex<Vec<StandardAuditLogPayload>>,
}

#[async_trait]
impl CallbackLogger for RecordingLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    async fn async_log_audit_event(
        &self,
        audit_log: &StandardAuditLogPayload,
    ) -> Result<(), LoggerError> {
        self.delivered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(audit_log.clone());
        Ok(())
    }
}

pub(super) struct NamedResolver {
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) loggers: Mutex<BTreeMap<String, Arc<dyn CallbackLogger>>>,
}

impl LoggerResolver for NamedResolver {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
        let logger = self
            .loggers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned();
        Ok(logger)
    }
}

pub(super) struct RecordingBuilder;

impl DedicatedAuditLoggerBuilder for RecordingBuilder {
    fn build(&self, _params: &JsonValue) -> Result<Arc<dyn CallbackLogger>, AuditLoggerBuildError> {
        Err(AuditLoggerBuildError::Build {
            source: CatalogError::Init {
                message: "the dedicated audit logger is not available here".to_owned(),
            },
        })
    }
}

pub(super) struct RecordingRepository {
    pub(super) inserted: Mutex<Vec<AuditLogRecord>>,
}

#[async_trait]
impl AuditLogRepository for RecordingRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

pub(super) struct FixedClock;

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(NOW).expect("the fixed clock is a timestamp")
    }
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
        before_value: None,
        updated_values: None,
    }
}

pub(super) struct AuditFixture {
    pub(super) service: AuditService,
    pub(super) resolver: Arc<NamedResolver>,
    pub(super) logger: Arc<RecordingLogger>,
    pub(super) repository: Arc<RecordingRepository>,
}

pub(super) fn start(settings: &AuditSettings) -> AuditFixture {
    let logger = Arc::new(RecordingLogger {
        name: "otel".to_owned(),
        delivered: Mutex::new(Vec::new()),
    });
    let mut available: BTreeMap<String, Arc<dyn CallbackLogger>> = BTreeMap::new();
    available.insert("otel".to_owned(), Arc::clone(&logger));
    let resolver = Arc::new(NamedResolver {
        calls: Mutex::new(Vec::new()),
        loggers: Mutex::new(available),
    });
    let repository = Arc::new(RecordingRepository {
        inserted: Mutex::new(Vec::new()),
    });
    let service = AuditService::start(AuditServiceDeps {
        settings: settings.clone(),
        callbacks: Vec::new(),
        delivery: audit_delivery(Arc::<NamedResolver>::clone(&resolver)),
        repository: Arc::<RecordingRepository>::clone(&repository),
        clock: Arc::new(FixedClock),
        new_id: Arc::new(|| "audit-id-1".to_owned()),
        worker_settings: worker_settings(),
    })
    .expect("the audit service starts");
    AuditFixture {
        service,
        resolver,
        logger,
        repository,
    }
}

fn audit_delivery(resolver: Arc<NamedResolver>) -> Arc<AuditDelivery> {
    Arc::new(AuditDelivery {
        named: NamedAuditCallbacks {
            resolver,
            dedicated: Arc::new(RecordingBuilder),
            cache: Mutex::new(BTreeMap::new()),
        },
        worker: LoggingWorker::start(worker_settings()).expect("the delivery worker starts"),
    })
}

pub(super) fn worker_settings() -> WorkerSettings {
    WorkerSettings {
        max_queue_size: 8,
        concurrency: 1,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(5),
        timeout_summary_window: Duration::from_secs(1),
    }
}

pub(super) async fn flush(service: &AuditService) {
    service.writes.flush().await;
    service.delivery.worker.flush().await;
}

pub(super) async fn install(
    fixture: &AuditFixture,
    hooks: &PythonHooks,
    loaded: &LoadedConfig,
) -> Result<(), AuditStartupError> {
    install_audit_callbacks(
        &fixture.service,
        hooks,
        loaded.configured.as_deref(),
        &loaded.policy,
    )
    .await
}

pub(super) fn resolve(fixture: &AuditFixture) -> Option<AuditCallback> {
    fixture.service.delivery.named.resolve("otel", None)
}

pub(super) fn calls(fixture: &AuditFixture) -> usize {
    fixture.resolver.calls.lock().unwrap().len()
}

pub(super) fn installed_names(service: &AuditService) -> Vec<String> {
    service
        .callbacks
        .lock()
        .unwrap()
        .iter()
        .map(entry_name)
        .collect()
}

pub(super) fn entry_name(entry: &AuditCallbackEntry) -> String {
    match entry {
        AuditCallbackEntry::Logger(logger) => logger.callback_name().to_owned(),
        AuditCallbackEntry::Named(name) => name.clone(),
        AuditCallbackEntry::Python(hook) => hook.value.clone(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8(bytes).expect("the captured output is UTF-8")
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
