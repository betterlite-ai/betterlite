use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secret_managers::secret_lookup::{SecretLookup, manager::ManagerState};
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient;
use betterlite_secret_managers::secret_read_dispatch::protocols::SyncSecretReader;
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

struct FixedSecret(Option<SecretString>);

impl SyncSecretReader for FixedSecret {
    fn sync_read_secret(&self, _: &str) -> Option<SecretString> {
        self.0.clone()
    }
}

struct UnusedPorts;

impl OidcHttp for UnusedPorts {
    fn get(&self, _: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the oracle fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for UnusedPorts {
    fn token(&self, _: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for UnusedPorts {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secrets(token: Option<&str>) -> SecretLookup {
    SecretLookup {
        manager: Some(ManagerState {
            client: SecretManagerClient::Vault(Arc::new(FixedSecret(
                token.map(SecretString::from),
            ))),
            settings: Some(SecretManagerSettings::default()),
            system: Some(KeyManagementSystem::HashicorpVault),
            access_mode: SecretAccessMode::ReadOnly,
            hosted_keys: None,
        }),
        oidc_http: Arc::new(UnusedPorts),
        azure_tokens: Arc::new(UnusedPorts),
        clock: Arc::new(UnusedPorts),
        oidc_cache: OidcTokenCache::default(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let message = fields.0.trim_matches('"').to_owned();
        if message.contains("PackedPolicySize") {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(message);
        }
    }
}

impl Warnings {
    pub(super) fn collected(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn warnings<F, R>(run: F) -> (R, Vec<String>)
where
    F: FnOnce() -> R,
{
    let warnings = Warnings::default();
    let subscriber = registry().with(warnings.clone());
    let value = with_default(subscriber, run);
    (value, warnings.collected())
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
}
