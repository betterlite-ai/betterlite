use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::custom_interface::{
    CustomOperationFailure, CustomParams, CustomSecretError, CustomSecretFuture,
    CustomSecretManager, CustomSecretOperation,
};
use crate::secret_lookup::SecretLookup;
use crate::secret_lookup::SecretValue;
use crate::secret_lookup::manager::ManagerState;
use crate::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use crate::secret_read_dispatch::SecretManagerClient;
use crate::secret_read_dispatch::protocols::{AwsSecretsManagerClient, AwsSecretsManagerError};
use crate::secret_read_dispatch::settings::SecretManagerSettings;
use crate::secret_types::{KeyManagementSystem, SecretAccessMode};

pub(super) enum Reply {
    Answer { status: u16, body: Vec<u8> },
    Failure,
}

pub(super) struct FakeHttp {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<OidcRequest>>,
}

impl FakeHttp {
    pub(super) fn with(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<OidcRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl OidcHttp for FakeHttp {
    fn get(&self, request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("a scripted OIDC reply");
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        match reply {
            Reply::Answer { status, body } => Ok(OidcResponse { status, body }),
            Reply::Failure => Err(OidcHttpError::Request {
                message: "scripted failure".to_owned(),
            }),
        }
    }
}

pub(super) struct ManualClock {
    base: Instant,
    offset: Mutex<Duration>,
    unix_seconds: Mutex<i64>,
}

impl ManualClock {
    pub(super) fn at(unix_seconds: i64) -> Self {
        Self {
            base: Instant::now(),
            offset: Mutex::new(Duration::ZERO),
            unix_seconds: Mutex::new(unix_seconds),
        }
    }

    pub(super) fn advance(&self, seconds: u64) {
        let mut offset = self.offset.lock().unwrap_or_else(PoisonError::into_inner);
        *offset = offset.saturating_add(Duration::from_secs(seconds));
        drop(offset);
        let mut unix = self
            .unix_seconds
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *unix = unix.saturating_add(i64::try_from(seconds).unwrap_or(i64::MAX));
    }
}

impl OidcClock for ManualClock {
    fn now(&self) -> Instant {
        self.base
            .checked_add(*self.offset.lock().unwrap_or_else(PoisonError::into_inner))
            .expect("a bounded clock offset")
    }

    fn unix_seconds(&self) -> i64 {
        *self
            .unix_seconds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Clone, Copy)]
pub(super) enum AzureReply {
    Token(&'static str),
    None,
    Failure(&'static str),
}

pub(super) struct FakeAzureTokens {
    reply: AzureReply,
    scopes: Mutex<Vec<String>>,
}

impl FakeAzureTokens {
    pub(super) fn with(reply: AzureReply) -> Self {
        Self {
            reply,
            scopes: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn scopes(&self) -> Vec<String> {
        self.scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl AzureOidcTokens for FakeAzureTokens {
    fn token(&self, scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        self.scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(scope.to_owned());
        match self.reply {
            AzureReply::Token(text) => Ok(Some(SecretString::from(text))),
            AzureReply::None => Ok(None),
            AzureReply::Failure(message) => Err(AzureOidcError::Provider {
                message: message.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum ManagerReply {
    Text(&'static str),
    Failure,
}

pub(super) struct FakeManager {
    reply: ManagerReply,
    reads: Mutex<Vec<String>>,
}

impl FakeManager {
    pub(super) fn with(reply: ManagerReply) -> Self {
        Self {
            reply,
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CustomSecretManager for FakeManager {
    fn secret_manager_name(&self) -> &'static str {
        "fake"
    }

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        optional_params: Option<&'borrowed CustomParams>,
    ) -> CustomSecretFuture<'borrowed, Option<SecretString>> {
        let result = self.sync_read_secret(secret_name, optional_params);
        Box::pin(async move { result })
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        _optional_params: Option<&CustomParams>,
    ) -> Result<Option<SecretString>, CustomSecretError> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(secret_name.to_owned());
        match self.reply {
            ManagerReply::Text(text) => Ok(Some(SecretString::from(text))),
            ManagerReply::Failure => Err(CustomSecretError::Operation(CustomOperationFailure {
                operation: CustomSecretOperation::SyncRead,
                exception_type: "RuntimeError".to_owned(),
                message: "scripted failure".to_owned(),
            })),
        }
    }
}

pub(super) struct FakeAwsSecrets {
    reply: Option<&'static str>,
}

impl FakeAwsSecrets {
    pub(super) const fn returning(reply: Option<&'static str>) -> Self {
        Self { reply }
    }
}

impl AwsSecretsManagerClient for FakeAwsSecrets {
    fn sync_read_secret(
        &self,
        _secret_name: &str,
        _primary_secret_name: Option<&str>,
    ) -> Result<Option<SecretString>, AwsSecretsManagerError> {
        Ok(self.reply.map(SecretString::from))
    }
}

pub(super) fn custom_client(reply: ManagerReply) -> SecretManagerClient {
    let manager = Arc::new(FakeManager::with(reply));
    let handle: Arc<dyn CustomSecretManager> = manager;
    SecretManagerClient::Custom(handle)
}

pub(super) fn custom_client_with(manager: &Arc<FakeManager>) -> SecretManagerClient {
    let concrete = Arc::clone(manager);
    let handle: Arc<dyn CustomSecretManager> = concrete;
    SecretManagerClient::Custom(handle)
}

pub(super) fn aws_secrets_client(reply: Option<&'static str>) -> SecretManagerClient {
    let concrete = Arc::new(FakeAwsSecrets::returning(reply));
    let handle: Arc<dyn AwsSecretsManagerClient> = concrete;
    SecretManagerClient::AwsSecretManager(handle)
}

pub(super) fn scripted_lookup(
    replies: Vec<Reply>,
    clock: &Arc<ManualClock>,
) -> (SecretLookup, Arc<FakeHttp>) {
    let http = Arc::new(FakeHttp::with(replies));
    let mut lookup = base_lookup();
    let http_handle = Arc::clone(&http);
    lookup.oidc_http = http_handle;
    let clock_handle = Arc::clone(clock);
    lookup.clock = clock_handle;
    (lookup, http)
}

pub(super) fn manager_state(
    client: SecretManagerClient,
    settings: Option<SecretManagerSettings>,
) -> ManagerState {
    ManagerState {
        client,
        settings,
        system: Some(KeyManagementSystem::Custom),
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    }
}

pub(super) fn base_lookup() -> SecretLookup {
    let http = Arc::new(FakeHttp::with(Vec::new()));
    let azure = Arc::new(FakeAzureTokens::with(AzureReply::Failure(
        "no Azure token provider",
    )));
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let http_handle: Arc<dyn OidcHttp> = http;
    let azure_handle: Arc<dyn AzureOidcTokens> = azure;
    let clock_handle: Arc<dyn OidcClock> = clock;
    SecretLookup {
        manager: None,
        oidc_http: http_handle,
        azure_tokens: azure_handle,
        clock: clock_handle,
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn text_value(value: Option<&SecretValue>) -> Option<String> {
    match value {
        Some(SecretValue::Text(text)) => Some(text.expose_secret().to_owned()),
        Some(SecretValue::Bool(_)) | None => None,
    }
}

pub(super) fn bool_value(value: Option<&SecretValue>) -> Option<bool> {
    match value {
        Some(SecretValue::Bool(flag)) => Some(*flag),
        Some(SecretValue::Text(_)) | None => None,
    }
}

pub(super) fn jwt(exp: i64) -> String {
    let payload = format!("{{\"exp\":{exp}}}");
    format!("e30.{}.c2ln", URL_SAFE_NO_PAD.encode(payload))
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("utf-8 log output")
    }
}

impl io::Write for LogBuffer {
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

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
