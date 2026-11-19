#![cfg(test)]

use std::io::Result as IoResult;
use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_config::loader::parse_view;
use betterlite_config::sections::includes::ConfigMap;
use betterlite_secret_managers::custom_interface::CustomParams;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::manager::ManagerState;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient;
use betterlite_secret_managers::secret_read_dispatch::protocols::SyncSecretReader;
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use secrecy::SecretString;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

pub(crate) fn config_from_yaml(source: &str) -> ConfigMap {
    let (config, ignored) = parse_view::<ConfigMap>(source).unwrap();
    assert!(ignored.is_empty());
    config
}

struct NoOidcHttp;

impl OidcHttp for NoOidcHttp {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "a config boot test has no OIDC provider".to_owned(),
        })
    }
}

struct NoAzureTokens;

impl AzureOidcTokens for NoAzureTokens {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

struct FixedClock;

impl OidcClock for FixedClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(crate) fn secret_lookup(manager: Option<ManagerState>) -> SecretLookup {
    SecretLookup {
        manager,
        oidc_http: Arc::new(NoOidcHttp),
        azure_tokens: Arc::new(NoAzureTokens),
        clock: Arc::new(FixedClock),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(crate) struct FixedReader(pub(crate) Option<&'static str>);

impl SyncSecretReader for FixedReader {
    fn sync_read_secret(&self, _secret_name: &str) -> Option<SecretString> {
        self.0.map(SecretString::from)
    }
}

pub(crate) fn managed_state(value: Option<&'static str>) -> ManagerState {
    ManagerState {
        client: SecretManagerClient::Vault(Arc::new(FixedReader(value))),
        settings: Some(SecretManagerSettings {
            primary_secret_name: None,
            optional_params: CustomParams::new(),
        }),
        system: Some(KeyManagementSystem::HashicorpVault),
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).unwrap()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_max_level(Level::TRACE)
        .without_time()
        .with_ansi(false)
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}
