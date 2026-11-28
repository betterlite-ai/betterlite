#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Request, StatusCode, header};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_cache::configuration::backend_construction::BackendConstructionError;
use betterlite_cache::configuration::type_not_supported::CacheTypeNotSupportedError;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_faults::reconnect::DbReconnect;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredValue,
};
use chrono::{DateTime, FixedOffset};
use secrecy::{ExposeSecret as _, SecretString};
use tower::ServiceExt as _;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::audit_events::callbacks::{
    AuditCallbackConfig, AuditCallbackDispatcher, AuditCallbackError, AuditCallbackFuture,
    AuditCallbackRegistry, AuditLogCallback, AuditTaskSpawner,
};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::AuditSettings;
use crate::audit_events::sink::{
    AuditClock, AuditEventSink, AuditLogRepository, AuditRepositoryError,
};
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::cache_settings_update::audit::{
    CacheSettingsAudit, CacheSettingsAuditEntry, CacheSettingsAuditError,
};
use crate::cache_settings_update::manager::{
    CacheParams, CacheReinit, CacheReinitError, CacheSettingsManager,
};
use crate::cache_settings_update::route::{
    CacheSettingsUpdateState, StoreModelInDbSettings, router,
};
use crate::cache_settings_update::store::CacheConfigRepository;

pub(crate) const TEST_SIGNING_KEY: &str = "cache-settings-update-signing-key";
pub(crate) const CHANGED_BY: &str = "litellm-changed-by";

pub(crate) fn cipher() -> Arc<DbCipher> {
    Arc::new(DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(TEST_SIGNING_KEY)),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    })
}

pub(crate) fn sealed(text: &str) -> String {
    let sealed = cipher()
        .encrypt_value(StoredValue::<()>::Text(SecretString::from(text)), None)
        .expect("the test value seals");
    match sealed {
        StoredValue::Text(value) => value.expose_secret().to_owned(),
        StoredValue::Other(()) => panic!("a string seals as text"),
    }
}

pub(crate) fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(crate) fn object(entries: &[(&str, JsonValue)]) -> JsonValue {
    JsonValue::Object(
        entries
            .iter()
            .map(|(key, entry)| ((*key).to_owned(), entry.clone()))
            .collect(),
    )
}

pub(crate) fn params(text: &str) -> CacheParams {
    serde_json::from_str(text).expect("the params decode")
}

pub(crate) fn stored_row(entries: &[(&str, JsonValue)]) -> JsonValue {
    JsonValue::String(serde_json::to_string(&object(entries)).expect("the stored row encodes"))
}

const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Original,
};

pub(crate) fn decrypt_all(value: &JsonValue) -> JsonValue {
    let Some(map) = value.as_object() else {
        return JsonValue::Null;
    };
    JsonValue::Object(
        map.iter()
            .map(|(key, entry)| (key.clone(), decrypt_entry(key, entry)))
            .collect(),
    )
}

fn decrypt_entry(key: &str, entry: &JsonValue) -> JsonValue {
    let JsonValue::String(text) = entry else {
        return entry.clone();
    };
    let stored = StoredValue::<()>::Text(SecretString::from(text.clone()));
    match cipher().decrypt_value(stored, key, KEEP_ORIGINAL) {
        Some(StoredValue::Text(plain)) => JsonValue::String(plain.expose_secret().to_owned()),
        Some(StoredValue::Other(())) | None => JsonValue::Null,
    }
}

pub(crate) fn database_error(error_type: ErrorType, message: &str) -> DbError {
    DbError {
        error_type,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

pub(crate) struct FakeRepository {
    reads: Mutex<VecDeque<Result<Option<JsonValue>, DbError>>>,
    upserted: Mutex<Vec<String>>,
    write_failure: Mutex<Option<DbError>>,
    read_count: AtomicUsize,
}

impl FakeRepository {
    pub(crate) fn new() -> Self {
        Self {
            reads: Mutex::new(VecDeque::new()),
            upserted: Mutex::new(Vec::new()),
            write_failure: Mutex::new(None),
            read_count: AtomicUsize::new(0),
        }
    }

    pub(crate) fn push_read(&self, result: Result<Option<JsonValue>, DbError>) {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(result);
    }

    pub(crate) fn fail_writes(&self, error: DbError) {
        *self
            .write_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(error);
    }

    pub(crate) fn read_count(&self) -> usize {
        self.read_count.load(Ordering::SeqCst)
    }

    pub(crate) fn upserted(&self) -> Vec<String> {
        self.upserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CacheConfigRepository for FakeRepository {
    async fn stored_cache_settings(&self) -> Result<Option<JsonValue>, DbError> {
        self.read_count.fetch_add(1, Ordering::SeqCst);
        let next = self
            .reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        next.unwrap_or(Ok(None))
    }

    async fn upsert_cache_settings(&self, settings_json: &str) -> Result<(), DbError> {
        let failure = self
            .write_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(error) = failure {
            return Err(error);
        }
        self.upserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(settings_json.to_owned());
        Ok(())
    }
}

pub(crate) fn fake_repository() -> (Arc<FakeRepository>, Arc<dyn CacheConfigRepository>) {
    let fake = Arc::new(FakeRepository::new());
    let erased: Arc<dyn CacheConfigRepository> = Arc::<FakeRepository>::clone(&fake);
    (fake, erased)
}

pub(crate) struct FakeReconnect {
    reconnects: bool,
    attempts: AtomicUsize,
    reasons: Mutex<Vec<String>>,
}

impl FakeReconnect {
    pub(crate) fn new(reconnects: bool) -> Self {
        Self {
            reconnects,
            attempts: AtomicUsize::new(0),
            reasons: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }

    pub(crate) fn reasons(&self) -> Vec<String> {
        self.reasons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl DbReconnect for FakeReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        _timeout_seconds: f64,
        _lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.reasons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(reason.to_owned());
        let reconnects = self.reconnects;
        Box::pin(async move { Ok(reconnects) })
    }
}

pub(crate) struct RecordingReinit {
    calls: Mutex<Vec<String>>,
    params: Mutex<Vec<CacheParams>>,
    failing: bool,
}

impl RecordingReinit {
    fn new(failing: bool) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            params: Mutex::new(Vec::new()),
            failing,
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn params(&self) -> Vec<CacheParams> {
        self.params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CacheReinit for RecordingReinit {
    fn init_cache(&self, params: &CacheParams) -> Result<(), CacheReinitError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("init_cache".to_owned());
        self.params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(params.clone());
        if self.failing {
            return Err(CacheReinitError::Init {
                source: BackendConstructionError::Unsupported(
                    CacheTypeNotSupportedError::NotImplemented {
                        cache_type: LiteLlmCacheType::Gcs,
                    },
                ),
            });
        }
        Ok(())
    }

    fn switch_on_response_caching(&self) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("switch_on_response_caching".to_owned());
    }
}

pub(crate) fn reinit() -> Arc<RecordingReinit> {
    Arc::new(RecordingReinit::new(false))
}

pub(crate) fn failing_reinit() -> Arc<RecordingReinit> {
    Arc::new(RecordingReinit::new(true))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditReply {
    Ok,
    Failed,
}

pub(crate) struct RecordingAudit {
    enabled: bool,
    reply: AuditReply,
    entries: Mutex<Vec<CacheSettingsAuditEntry>>,
}

impl RecordingAudit {
    pub(crate) fn new(enabled: bool, reply: AuditReply) -> Self {
        Self {
            enabled,
            reply,
            entries: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn entries(&self) -> Vec<CacheSettingsAuditEntry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl CacheSettingsAudit for RecordingAudit {
    fn enabled(&self) -> bool {
        self.enabled
    }

    async fn write(&self, entry: CacheSettingsAuditEntry) -> Result<(), CacheSettingsAuditError> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
        match self.reply {
            AuditReply::Ok => Ok(()),
            AuditReply::Failed => Err(CacheSettingsAuditError::Write {
                message: "the audit table is unreachable".to_owned(),
            }),
        }
    }
}

pub(crate) fn audit(enabled: bool) -> Arc<RecordingAudit> {
    Arc::new(RecordingAudit::new(enabled, AuditReply::Ok))
}

pub(crate) fn failing_audit() -> Arc<RecordingAudit> {
    Arc::new(RecordingAudit::new(true, AuditReply::Failed))
}

#[derive(Default)]
pub(crate) struct CollectingSpawner {
    tasks: Mutex<Vec<AuditCallbackFuture>>,
}

impl CollectingSpawner {
    pub(crate) fn take(&self) -> Vec<AuditCallbackFuture> {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl AuditTaskSpawner for CollectingSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(task);
    }
}

pub(crate) async fn run_tasks(spawner: &CollectingSpawner) {
    for task in spawner.take() {
        task.await;
    }
}

struct StoreFlag(bool);

impl StoreModelInDbSettings for StoreFlag {
    fn store_model_in_db(&self) -> bool {
        self.0
    }
}

pub(crate) struct Harness {
    pub(crate) manager: Arc<CacheSettingsManager>,
    pub(crate) spawner: Arc<CollectingSpawner>,
    pub(crate) state: CacheSettingsUpdateState,
}

impl Harness {
    pub(crate) fn disable_store_model_in_db(&mut self) {
        self.state.store_model_in_db = Arc::new(StoreFlag(false));
    }
}

pub(crate) fn harness(
    repository: Option<Arc<dyn CacheConfigRepository>>,
    reinit: Arc<RecordingReinit>,
    audit: Arc<RecordingAudit>,
) -> Harness {
    let manager = Arc::new(CacheSettingsManager::default());
    let spawner = Arc::new(CollectingSpawner::default());
    Harness {
        manager: Arc::clone(&manager),
        spawner: Arc::clone(&spawner),
        state: CacheSettingsUpdateState {
            repository,
            cipher: cipher(),
            manager,
            reinit,
            audit,
            spawner,
            store_model_in_db: Arc::new(StoreFlag(true)),
        },
    }
}

pub(crate) fn master_key() -> AuthContext {
    master_caller()
}

pub(crate) fn platform_admin() -> AuthContext {
    caller(
        "platform-admin",
        vec![Role::PlatformAdmin],
        SourceKey::default(),
    )
}

pub(crate) fn non_admin() -> AuthContext {
    caller("internal-user", Vec::new(), SourceKey::default())
}

pub(crate) fn user_key(user_id: &str, api_key: &str) -> AuthContext {
    let fields = BTreeMap::from([
        ("user_id".to_owned(), JsonData::String(user_id.to_owned())),
        ("api_key".to_owned(), JsonData::String(api_key.to_owned())),
    ]);
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the test key builds");
    caller(
        "key-subject",
        vec![Role::PlatformAdmin],
        SourceKey(Some(key)),
    )
}

fn caller(subject: &str, roles: Vec<Role>, source_key: SourceKey) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key,
        },
    }
}

pub(crate) fn request_body(settings: JsonValue) -> String {
    let body = object(&[("cache_settings", settings)]);
    serde_json::to_string(&body).expect("the request encodes")
}

// The request future exceeds clippy's large-futures ceiling, so it is boxed here and callers await a pointer.
pub(crate) fn post<'test>(
    harness: &'test Harness,
    auth: Option<AuthContext>,
    body: &'test str,
) -> Pin<Box<dyn Future<Output = (StatusCode, String)> + Send + 'test>> {
    post_with_header(harness, auth, body, None)
}

pub(crate) fn post_with_header<'test>(
    harness: &'test Harness,
    auth: Option<AuthContext>,
    body: &'test str,
    changed_by: Option<&'test [u8]>,
) -> Pin<Box<dyn Future<Output = (StatusCode, String)> + Send + 'test>> {
    Box::pin(async move {
        let application = router(harness.state.clone());
        let mut builder = Request::builder()
            .method("POST")
            .uri("/cache/settings")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(value) = changed_by {
            builder = builder.header(
                CHANGED_BY,
                HeaderValue::from_bytes(value).expect("the test header builds"),
            );
        }
        let mut request = builder
            .body(Body::from(body.to_owned()))
            .expect("the test request builds");
        if let Some(context) = auth {
            request.extensions_mut().insert(context);
        }
        let response = Box::pin(application.oneshot(request))
            .await
            .expect("the router answers");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body reads");
        (
            status,
            String::from_utf8(bytes.to_vec()).expect("the body is utf-8"),
        )
    })
}

#[derive(Clone, Default)]
pub(crate) struct LogSink(Arc<Mutex<Vec<u8>>>);

impl LogSink {
    pub(crate) fn text(&self) -> String {
        let captured = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(captured).expect("the log is utf-8")
    }
}

impl io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn capture_logs() -> (LogSink, DefaultGuard) {
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (sink, set_default(subscriber))
}

struct FakeAuditSettings {
    enabled: bool,
}

impl AuditSettings for FakeAuditSettings {
    fn is_audit_logging_enabled(&self, _explicit: Option<bool>) -> bool {
        self.enabled
    }

    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig] {
        &[]
    }

    fn s3_audit_callback_params(&self) -> Option<&JsonValue> {
        None
    }
}

#[derive(Default)]
pub(crate) struct CapturingRepository {
    records: Mutex<Vec<AuditLogRecord>>,
}

impl CapturingRepository {
    pub(crate) fn records(&self) -> Vec<AuditLogRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AuditLogRepository for CapturingRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

struct NoCallbacks;

impl AuditCallbackRegistry for NoCallbacks {
    fn resolve(&self, _name: &str) -> Option<Arc<dyn AuditLogCallback>> {
        None
    }

    fn build_dedicated_s3_logger(
        &self,
        _params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError> {
        Err(AuditCallbackError::Failed)
    }
}

struct FixedClock;

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2024-05-01T00:00:00Z").expect("the fixed timestamp parses")
    }
}

pub(crate) fn sink(enabled: bool) -> (Arc<AuditEventSink>, Arc<CapturingRepository>) {
    let settings: Arc<dyn AuditSettings> = Arc::new(FakeAuditSettings { enabled });
    let callbacks = Arc::new(AuditCallbackDispatcher::new(
        Arc::new(NoCallbacks),
        Arc::new(CollectingSpawner::default()),
    ));
    let repository = Arc::new(CapturingRepository::default());
    let erased: Arc<dyn AuditLogRepository> = Arc::<CapturingRepository>::clone(&repository);
    let sink = AuditEventSink {
        settings,
        callbacks,
        repository: erased,
        clock: Arc::new(FixedClock),
        new_id: Arc::new(|| "audit-id".to_owned()),
    };
    (Arc::new(sink), repository)
}
