use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::sso_mappings::repository::SsoSettingsRows;
use betterlite_config::loader::EnvironmentLookup as _;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::websearch_request_hooks::Settings as WebsearchParams;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_faults::reconnect::DbReconnect;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::postgres_store::errors::DbError as NativeDbError;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secret_managers::secret_types::KeyManagementSystem;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use crate::config_override_support::environment::RuntimeEnvironment;
use crate::config_override_support::models::OverrideKind;
use crate::config_override_support::state::{
    OverrideRuntimeState, SecretManagerControl, SecretManagerInitError,
};
use crate::db_row_decrypt::warnings::Warnings;
use crate::db_settings_reload::SettingsReloader;
use crate::db_settings_reload::ports::{
    NoReconnect, SemanticFilterReload, SettingsRows, WebsearchInterception,
};

const KEY: &str = "b8383-test-key";

pub(crate) fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(KEY.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn sealed(cipher: &DbCipher, text: &str) -> String {
    let stored = cipher
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

pub(crate) fn fault(message: &str) -> DbError {
    DbError {
        error_type: ErrorType::Connect,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

#[derive(Default)]
pub(crate) struct FakeRows {
    pub litellm_settings: Option<JsonValue>,
    pub vault: Option<JsonValue>,
    pub cyberark: Option<JsonValue>,
    pub fail: Option<DbError>,
    // fail only the first read, so a retried read can succeed
    pub fail_once: bool,
    pub calls: AtomicUsize,
}

impl FakeRows {
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(crate) fn litellm(settings: JsonValue) -> Self {
        Self::rows(Some(settings), None, None, None)
    }

    pub(crate) fn failing(error: DbError) -> Self {
        Self::rows(None, None, None, Some(error))
    }

    pub(crate) fn vault(config: JsonValue) -> Self {
        Self::rows(None, Some(config), None, None)
    }

    pub(crate) fn cyberark(config: JsonValue) -> Self {
        Self::rows(None, None, Some(config), None)
    }

    fn rows(
        litellm_settings: Option<JsonValue>,
        vault: Option<JsonValue>,
        cyberark: Option<JsonValue>,
        fail: Option<DbError>,
    ) -> Self {
        Self {
            litellm_settings,
            vault,
            cyberark,
            fail,
            fail_once: false,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl SettingsRows for FakeRows {
    async fn litellm_settings(&self) -> Result<Option<JsonValue>, DbError> {
        match &self.fail {
            Some(error) => Err(error.clone()),
            None => Ok(self.litellm_settings.clone()),
        }
    }

    async fn config_override(&self, kind: OverrideKind) -> Result<Option<JsonValue>, DbError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = &self.fail {
            if !self.fail_once || call == 0 {
                return Err(error.clone());
            }
        }
        Ok(match kind {
            OverrideKind::HashicorpVault => self.vault.clone(),
            OverrideKind::CyberArk => self.cyberark.clone(),
        })
    }
}

pub(crate) enum SsoReply {
    Row(Option<String>),
    Transport,
    // transport on the first read only, the row on every later one
    Flaky(String),
    Failed,
}

pub(crate) struct FakeSso {
    pub reply: SsoReply,
    pub calls: AtomicUsize,
}

impl FakeSso {
    pub(crate) fn new(reply: SsoReply) -> Self {
        Self {
            reply,
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl SsoSettingsRows for FakeSso {
    async fn stored_sso_settings(&self) -> Result<Option<String>, TableError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.reply {
            SsoReply::Row(text) => Ok(text.clone()),
            SsoReply::Transport => Err(TableError::Database(NativeDbError::NoDbConnected)),
            SsoReply::Flaky(_) if call == 0 => {
                Err(TableError::Database(NativeDbError::NoDbConnected))
            }
            SsoReply::Flaky(text) => Ok(Some(text.clone())),
            SsoReply::Failed => Err(TableError::MissingRow),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ReconnectResult {
    Reconnected,
    Declined,
    Failed,
}

pub(crate) struct FakeReconnect {
    pub result: ReconnectResult,
    pub reasons: Arc<Mutex<Vec<String>>>,
}

impl FakeReconnect {
    pub(crate) fn new(result: ReconnectResult) -> (Self, Arc<Mutex<Vec<String>>>) {
        let reasons = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                result,
                reasons: reasons.clone(),
            },
            reasons,
        )
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
        self.reasons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(reason.to_owned());
        match self.result {
            ReconnectResult::Reconnected => Box::pin(async { Ok(true) }),
            ReconnectResult::Declined => Box::pin(async { Ok(false) }),
            ReconnectResult::Failed => Box::pin(async { Err(fault("the reconnect failed")) }),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ControlLog {
    pub active: Arc<Mutex<Option<KeyManagementSystem>>>,
    pub initialized: Arc<Mutex<Vec<KeyManagementSystem>>>,
    pub dropped: Arc<AtomicUsize>,
    pub fail: Arc<AtomicBool>,
}

impl ControlLog {
    pub(crate) fn initialized(&self) -> Vec<KeyManagementSystem> {
        self.initialized
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn dropped(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }

    pub(crate) fn active(&self) -> Option<KeyManagementSystem> {
        *self.active.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn set_fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }

    pub(crate) fn reset(&self) {
        self.initialized
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.dropped.store(0, Ordering::SeqCst);
        self.fail.store(false, Ordering::SeqCst);
    }
}

struct FakeControl {
    control: ControlLog,
}

impl SecretManagerControl for FakeControl {
    fn active_manager(&self) -> Option<KeyManagementSystem> {
        self.control.active()
    }

    fn drop_active_manager(&mut self) {
        self.control.dropped.fetch_add(1, Ordering::SeqCst);
        *self
            .control
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn initialize(&mut self, kind: KeyManagementSystem) -> Result<(), SecretManagerInitError> {
        self.control
            .initialized
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(kind);
        if self.control.fail.load(Ordering::SeqCst) {
            return Err(SecretManagerInitError {
                message: "the fake secret manager is down".to_owned(),
            });
        }
        *self
            .control
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(kind);
        Ok(())
    }
}

pub(crate) fn reloader_with<Client>(
    control: &ControlLog,
    reconnect: Option<Client>,
) -> SettingsReloader<Client>
where
    Client: Send,
{
    SettingsReloader {
        cipher: cipher(),
        environment: RuntimeEnvironment::default(),
        control: Box::new(FakeControl {
            control: control.clone(),
        }),
        warnings: Warnings::default(),
        state: OverrideRuntimeState::default(),
        reconnect,
        last_websearch: None,
        last_semantic_filter: None,
    }
}

pub(crate) fn reloader(control: &ControlLog) -> SettingsReloader<NoReconnect> {
    reloader_with(control, None)
}

pub(crate) fn env_value<Client>(reloader: &SettingsReloader<Client>, name: &str) -> Option<String>
where
    Client: Send,
{
    reloader.environment.lookup(name)
}

#[derive(Default)]
pub(crate) struct FakeCallbacks {
    pub registered: AtomicBool,
    pub applied: Mutex<Vec<Option<WebsearchParams>>>,
}

impl FakeCallbacks {
    pub(crate) fn applied(&self) -> Vec<Option<WebsearchParams>> {
        self.applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl WebsearchInterception for FakeCallbacks {
    fn is_registered(&self) -> bool {
        self.registered.load(Ordering::SeqCst)
    }

    fn apply(&self, params: Option<&WebsearchParams>) {
        self.registered.store(params.is_some(), Ordering::SeqCst);
        self.applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(params.cloned());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RebuildCall {
    pub enabled: bool,
    pub has_router: bool,
}

#[derive(Default)]
pub(crate) struct FakeFilter {
    pub ready: AtomicBool,
    pub registers: AtomicBool,
    pub rebuilds: Mutex<Vec<RebuildCall>>,
}

impl FakeFilter {
    pub(crate) fn rebuilds(&self) -> Vec<RebuildCall> {
        self.rebuilds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl<Router: ?Sized + Sync> SemanticFilterReload<Router> for FakeFilter {
    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    async fn rebuild(&self, enabled: bool, router: Option<&Router>) -> bool {
        self.rebuilds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RebuildCall {
                enabled,
                has_router: router.is_some(),
            });
        self.registers.load(Ordering::SeqCst)
    }
}

pub(crate) struct TestRouter {
    pub name: &'static str,
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }

    pub(crate) fn count(&self, needle: &str) -> usize {
        self.contents().matches(needle).count()
    }
}

impl io::Write for Captured {
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

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (Captured, DefaultGuard) {
    let buffer = Captured::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::TRACE)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
