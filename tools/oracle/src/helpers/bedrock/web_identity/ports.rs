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
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
struct Ports(Option<SecretString>);
impl SyncSecretReader for Ports {
    fn sync_read_secret(&self, _: &str) -> Option<SecretString> {
        self.0.clone()
    }
}
impl OidcHttp for Ports {
    fn get(&self, _: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the oracle fixture has no OIDC transport".to_owned(),
        })
    }
}
impl AzureOidcTokens for Ports {
    fn token(&self, _: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}
impl OidcClock for Ports {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn unix_seconds(&self) -> i64 {
        0
    }
}
pub(super) fn secrets(token: Option<&str>) -> SecretLookup {
    let client = SecretManagerClient::Vault(Arc::new(Ports(token.map(SecretString::from))));
    SecretLookup {
        manager: Some(ManagerState {
            client,
            settings: Some(SecretManagerSettings::default()),
            system: Some(KeyManagementSystem::HashicorpVault),
            access_mode: SecretAccessMode::ReadOnly,
            hosted_keys: None,
        }),
        oidc_http: Arc::new(Ports(None)),
        azure_tokens: Arc::new(Ports(None)),
        clock: Arc::new(Ports(None)),
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
pub(super) fn warnings<F, R>(run: F) -> (R, Vec<String>)
where
    F: FnOnce() -> R,
{
    let warnings = Warnings::default();
    let subscriber = registry().with(warnings.clone());
    let value = with_default(subscriber, run);
    let collected = warnings
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, collected)
}
