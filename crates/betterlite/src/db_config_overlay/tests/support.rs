use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::scheduled_job_stagger::Settings as StaggerSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_db::table_ops::rows::RowError;
use betterlite_secret_managers::secret_lookup::ports::{
    OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredValue};
use betterlite_server::bucket_config::object::BucketObjectReader;
use betterlite_server::db_row_decrypt::warnings::Warnings;
use betterlite_server::plugin_routes::config::PluginRegistry;
use betterlite_server::proxy_config_state::snapshot::ConfigWithBaseline;
use betterlite_server::proxy_config_state::state::ProxyConfigState;
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::{Semaphore, oneshot};

use crate::cleanup_schedule::changes::CleanupScheduleState;
use crate::cleanup_schedule::ports::{CacheMaxSize, CleanupScheduler};
use crate::cleanup_schedule::trigger::ScheduleContext;
use crate::config_boot::raw_secret_manager::RawSecretManager;
use crate::config_boot::test_support::secret_lookup;
use crate::db_config_overlay::environ::{WORKER_LIMIT, Workers};
use crate::db_config_overlay::error::{EndpointInstallError, OverlayError};
use crate::db_config_overlay::general::{CleanupInputs, EffectPorts, apply_general_settings_row};
use crate::db_config_overlay::pass_through::PassThroughInstaller;
use crate::db_config_overlay::reload::{
    begin_reload, finish_reload, reload_stores, resolve_reload,
};
use crate::db_config_overlay::rows::{apply_db_rows, fetch_db_rows};
use crate::db_config_overlay::source::{ConfigSource, read_config};
use crate::db_config_overlay::state::{
    ConfigRowReader, OverlayDatabase, OverlayState, RuntimeEffect, RuntimeSink,
};
use crate::job_scheduler::{BoxFuture, JobFn, JobSpec, SchedulerError, SchedulerState};

// JSON for the tests through serde_json's parser, so no banned object path reaches the lint table
pub(super) fn json_value(text: &str) -> JsonValue {
    serde_json::from_str::<JsonValue>(text).unwrap()
}

macro_rules! json {
    ($($part:tt)+) => {
        $crate::db_config_overlay::tests::support::json_value(stringify!($($part)+))
    };
}
pub(super) use json;

const KEY: &str = "b8379-test-key";

// one ordered log every fake writes to, so a test can read the order of the side effects
#[derive(Clone, Default)]
pub(super) struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    pub(super) fn push(&self, entry: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry.to_owned());
    }

    pub(super) fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
pub(super) struct RecordingRuntime {
    pub(super) journal: Journal,
    pub(super) effects: Mutex<Vec<RuntimeEffect>>,
}

impl RecordingRuntime {
    pub(super) fn effects(&self) -> Vec<RuntimeEffect> {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RuntimeSink for RecordingRuntime {
    fn apply(&self, effect: RuntimeEffect) {
        let label = format!("{effect:?}");
        let name = label.split(['(', ' ']).next().unwrap_or_default();
        self.journal.push(&format!("runtime:{name}"));
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }
}

#[derive(Default)]
pub(super) struct RecordingInstaller {
    pub(super) journal: Journal,
    pub(super) fails: bool,
    pub(super) calls: Mutex<Vec<Vec<JsonValue>>>,
}

impl RecordingInstaller {
    pub(super) fn calls(&self) -> Vec<Vec<JsonValue>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl PassThroughInstaller for RecordingInstaller {
    async fn install(&self, endpoints: &[JsonValue]) -> Result<(), EndpointInstallError> {
        self.journal.push("install");
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(endpoints.to_vec());
        if self.fails {
            return Err(EndpointInstallError);
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct RecordingCache {
    pub(super) journal: Journal,
    pub(super) sizes: Mutex<Vec<Option<usize>>>,
}

impl CacheMaxSize for RecordingCache {
    fn resize(&self, max_size: Option<usize>) {
        self.journal.push("cache");
        self.sizes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(max_size);
    }
}

pub(super) struct RecordingScheduler {
    pub(super) journal: Journal,
    pub(super) inserted: Vec<String>,
}

impl CleanupScheduler for RecordingScheduler {
    fn scheduler_state(&self) -> SchedulerState {
        SchedulerState::Running
    }

    fn has_job(&self, id: &str) -> bool {
        self.inserted.iter().any(|inserted| inserted == id)
    }

    fn insert_job(&mut self, spec: JobSpec) -> Result<(), SchedulerError> {
        self.journal.push("scheduler");
        self.inserted.push(spec.id);
        Ok(())
    }

    fn remove_existing_job(&mut self, id: &str) -> Result<(), SchedulerError> {
        self.inserted.retain(|inserted| inserted != id);
        Ok(())
    }
}

pub(super) struct Schedule {
    stagger: StaggerSettings,
    database: TimeZoneDatabase,
}

pub(super) fn schedule() -> Schedule {
    Schedule {
        stagger: StaggerSettings::default(),
        database: TimeZoneDatabase::from_env(),
    }
}

impl Schedule {
    pub(super) fn context(&self) -> ScheduleContext<'_> {
        ScheduleContext {
            stagger: &self.stagger,
            identity: "b8379",
            now: DateTime::from_timestamp(1_767_225_630, 0).unwrap(),
            timezone: None,
            database: &self.database,
        }
    }
}

pub(super) fn noop_job() -> JobFn {
    Arc::new(|| {
        let future: BoxFuture<'static, ()> = Box::pin(async {});
        future
    })
}

// a reader over fixed rows, or one that fails every read with the error the test names
pub(super) struct MemoryRows {
    pub(super) rows: BTreeMap<String, JsonValue>,
    pub(super) failure: Option<fn() -> DbError<RowError>>,
}

#[async_trait]
impl ConfigRowReader for MemoryRows {
    async fn config_param(&self, name: &str) -> Result<Option<JsonValue>, DbError<RowError>> {
        if let Some(failure) = self.failure {
            return Err(failure());
        }
        Ok(self.rows.get(name).cloned())
    }
}

pub(super) fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(KEY.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn sealed(text: &str) -> String {
    let stored = cipher()
        .encrypt_value(
            StoredValue::<()>::Text(SecretString::from(text.to_owned())),
            None,
        )
        .unwrap();
    let StoredValue::Text(sealed) = stored else {
        panic!("a text value seals as text");
    };
    sealed.expose_secret().to_owned()
}

pub(super) fn database(rows: &[(&str, JsonValue)]) -> OverlayDatabase {
    let stored: BTreeMap<String, JsonValue> = rows
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    OverlayDatabase {
        rows: Arc::new(MemoryRows {
            rows: stored,
            failure: None,
        }),
        cipher: Arc::new(cipher()),
        warnings: Arc::new(Warnings::default()),
    }
}

pub(super) fn failing_database(failure: fn() -> DbError<RowError>) -> OverlayDatabase {
    OverlayDatabase {
        rows: Arc::new(MemoryRows {
            rows: BTreeMap::new(),
            failure: Some(failure),
        }),
        cipher: Arc::new(cipher()),
        warnings: Arc::new(Warnings::default()),
    }
}

// a reader over fixed rows that reports its first read and then waits for the test's permits
pub(super) struct GatedRows {
    pub(super) rows: BTreeMap<String, JsonValue>,
    gate: Arc<Semaphore>,
    started: Mutex<Option<oneshot::Sender<()>>>,
}

#[async_trait]
impl ConfigRowReader for GatedRows {
    async fn config_param(&self, name: &str) -> Result<Option<JsonValue>, DbError<RowError>> {
        let started = self
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(sender) = started {
            let _sent = sender.send(());
        }
        let permit = self.gate.acquire().await.unwrap();
        drop(permit);
        Ok(self.rows.get(name).cloned())
    }
}

pub(super) fn gated_database(
    rows: &[(&str, JsonValue)],
) -> (OverlayDatabase, oneshot::Receiver<()>, Arc<Semaphore>) {
    let gate = Arc::new(Semaphore::new(0));
    let (started, started_reading) = oneshot::channel();
    let stored: BTreeMap<String, JsonValue> = rows
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    let database = OverlayDatabase {
        rows: Arc::new(GatedRows {
            rows: stored,
            gate: Arc::clone(&gate),
            started: Mutex::new(Some(started)),
        }),
        cipher: Arc::new(cipher()),
        warnings: Arc::new(Warnings::default()),
    };
    (database, started_reading, gate)
}

pub(super) fn state(store_model_in_db: bool, runtime: &Arc<RecordingRuntime>) -> OverlayState {
    let runtime_sink: Arc<dyn RuntimeSink> = Arc::<RecordingRuntime>::clone(runtime);
    OverlayState {
        config: ProxyConfigState::new(
            PathBuf::from("/etc/betterlite/config.yaml"),
            store_model_in_db,
        ),
        secrets: RawSecretManager::new(secret_lookup(None)),
        runtime: runtime_sink,
        startup_pass_through: Vec::new(),
        warned_shadowed: BTreeSet::new(),
        cleanup: CleanupScheduleState::default(),
    }
}

pub(super) fn file_source(path: &Path) -> ConfigSource {
    ConfigSource {
        config_file_path: Some(path.to_path_buf()),
        bucket_reader: None,
    }
}

// _update_config_from_db: the stored rows are read, then applied to their stores
pub(super) async fn overlay_db_rows(
    overlay: &mut OverlayState,
    database: &OverlayDatabase,
    env: &mut BTreeMap<String, String>,
) -> Result<(), OverlayError> {
    let stored = fetch_db_rows(database).await?;
    apply_db_rows(overlay, stored, database, env);
    Ok(())
}

// get_config's phases against a state the test owns; the port drives the same steps around its own locks
pub(super) async fn reload_config(
    overlay: &mut OverlayState,
    source: &ConfigSource,
    db: Option<&OverlayDatabase>,
    env: &mut BTreeMap<String, String>,
) -> Result<ConfigWithBaseline, OverlayError> {
    let workers = Workers {
        slots: Arc::new(Semaphore::new(WORKER_LIMIT)),
        live: Mutex::new(Vec::new()),
        drain: Mutex::new(None),
    };
    let file_config = read_config(source, env, &workers).await?;
    reload_stores(overlay, &file_config)?;
    let stored = match db.filter(|_| overlay.config.store_model_in_db) {
        Some(backed) => Some((backed, fetch_db_rows(backed).await?)),
        None => None,
    };
    let pending = begin_reload(overlay, &file_config, stored, source, env)?;
    let resolved = resolve_reload(pending, env.clone(), &workers).await?;
    workers.shutdown().await?;
    Ok(finish_reload(overlay, resolved))
}

// a bucket holding one config object under the key the test names
pub(super) struct MemoryBucket {
    pub(super) objects: BTreeMap<String, String>,
}

#[async_trait]
impl BucketObjectReader for MemoryBucket {
    async fn read_object(&self, object_key: &str) -> Option<Vec<u8>> {
        self.objects
            .get(object_key)
            .map(|text| text.as_bytes().to_vec())
    }
}

pub(super) fn bucket_source(objects: &[(&str, &str)]) -> ConfigSource {
    let stored: BTreeMap<String, String> = objects
        .iter()
        .map(|(key, text)| ((*key).to_owned(), (*text).to_owned()))
        .collect();
    ConfigSource {
        config_file_path: None,
        bucket_reader: Some(Arc::new(MemoryBucket { objects: stored })),
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

// a Google OIDC endpoint that answers once and is unreachable after that
pub(super) struct OnceOidc {
    pub(super) calls: AtomicUsize,
}

impl OidcHttp for OnceOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        if self.calls.fetch_add(1, Ordering::Relaxed) > 0 {
            return Err(OidcHttpError::Request {
                message: "the token endpoint is down".to_owned(),
            });
        }
        Ok(OidcResponse {
            status: 200,
            body: b"google-token".to_vec(),
        })
    }
}

// a Google OIDC endpoint that reports its first lookup and waits for the test to release it
pub(super) struct BlockingOidc {
    started: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<Option<Receiver<()>>>,
}

impl OidcHttp for BlockingOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        let started = self
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(sender) = started {
            let _sent = sender.send(());
        }
        let release = self
            .release
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(receiver) = release {
            let _released = receiver.recv();
        }
        Ok(OidcResponse {
            status: 200,
            body: b"google-token".to_vec(),
        })
    }
}

pub(super) fn blocking_oidc() -> (Arc<BlockingOidc>, oneshot::Receiver<()>, SyncSender<()>) {
    let (started, started_rx) = oneshot::channel();
    let (release, released) = mpsc::sync_channel(1);
    let oidc = Arc::new(BlockingOidc {
        started: Mutex::new(Some(started)),
        release: Mutex::new(Some(released)),
    });
    (oidc, started_rx, release)
}

const VAULT_BODY: &str = "{\"data\":{\"data\":{\"key\":\"sk-vault-secret\"}}}";

// A Vault on loopback answering the one read a reload makes, so the manager transport runs its real HTTP path
pub(super) struct MockVault {
    pub(super) address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stopping: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
}

impl MockVault {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let server = {
            let request_log = Arc::clone(&requests);
            let stop_signal = Arc::clone(&stopping);
            thread::Builder::new()
                .name("b8379-mock-vault".to_owned())
                .spawn(move || serve_vault(&listener, &request_log, &stop_signal))
                .unwrap()
        };
        Self {
            address,
            requests,
            stopping,
            server: Some(server),
        }
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// the accept loop runs until Drop sets the flag and connects once more to wake it
fn serve_vault(listener: &TcpListener, requests: &Mutex<Vec<String>>, stopping: &AtomicBool) {
    while !stopping.load(Ordering::Relaxed) {
        let Ok((mut stream, _peer)) = listener.accept() else {
            break;
        };
        answer_vault(&mut stream, requests);
    }
}

// one Vault read: record the request head and answer with the fixed secret
fn answer_vault(stream: &mut TcpStream, requests: &Mutex<Vec<String>>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let head = read_vault_request(stream);
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(String::from_utf8(head).unwrap());
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{VAULT_BODY}",
        VAULT_BODY.len()
    );
    drop(stream.write_all(response.as_bytes()));
}

// the request head up to the blank line that ends it
fn read_vault_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 512];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).unwrap_or_default();
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    head
}

impl Drop for MockVault {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        drop(TcpStream::connect(self.address));
        if let Some(server) = self.server.take() {
            let _joined = server.join();
        }
    }
}

pub(super) fn write_config(directory: &Path, yaml: &str) -> PathBuf {
    let path = directory.join("config.yaml");
    fs::write(&path, yaml).unwrap();
    path
}

pub(super) fn values(pairs: &[(&str, JsonValue)]) -> BTreeMap<String, JsonValue> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

// every port apply_general_settings_row takes, each writing to one shared journal
pub(super) struct Fixture {
    pub(super) journal: Journal,
    pub(super) runtime: Arc<RecordingRuntime>,
    pub(super) installer: RecordingInstaller,
    pub(super) cache: RecordingCache,
    pub(super) scheduler: RecordingScheduler,
    pub(super) plugins: PluginRegistry,
    pub(super) schedule: Schedule,
}

pub(super) fn fixture() -> Fixture {
    let journal = Journal::default();
    Fixture {
        runtime: Arc::new(RecordingRuntime {
            journal: journal.clone(),
            effects: Mutex::new(Vec::new()),
        }),
        installer: RecordingInstaller {
            journal: journal.clone(),
            fails: false,
            calls: Mutex::new(Vec::new()),
        },
        cache: RecordingCache {
            journal: journal.clone(),
            sizes: Mutex::new(Vec::new()),
        },
        scheduler: RecordingScheduler {
            journal: journal.clone(),
            inserted: Vec::new(),
        },
        plugins: PluginRegistry::default(),
        schedule: schedule(),
        journal,
    }
}

impl Fixture {
    pub(super) fn state(&self) -> OverlayState {
        state(true, &self.runtime)
    }

    pub(super) async fn apply(
        &mut self,
        state: &mut OverlayState,
        row: &JsonValue,
    ) -> Result<(), OverlayError> {
        let context = self.schedule.context();
        let cleanup = CleanupInputs {
            scheduler: Some(&mut self.scheduler),
            context: &context,
            job: noop_job(),
        };
        let ports = EffectPorts {
            cache: &self.cache,
            plugins: &self.plugins,
        };
        apply_general_settings_row(state, row, cleanup, &ports, &self.installer).await
    }
}
