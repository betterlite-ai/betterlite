use std::collections::BTreeMap;
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use betterlite_config::sections::includes::ConfigMap;
use betterlite_config::sections::settings::JsonValue;
use betterlite_models::controls::multipart::DEFAULT_MAX_RECURSE_DEPTH;
use betterlite_secret_managers::secret_lookup::{SecretLookup, SecretValue};
use betterlite_secret_managers::secret_lookup::{manager::ManagerState, ports::OidcTokenCache};
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient as Client;
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use secrecy::ExposeSecret as _;
use tokio::sync::{Semaphore, oneshot};

use super::error::OverlayError;

const ENVIRONMENT_PREFIX: &str = "os.environ/";
pub const WORKER_LIMIT: usize = 4;

// What one resolution returns: the config, and the OIDC tokens its lookups fetched, which outlive the worker.
pub type ResolvedConfig = (ConfigMap, OidcTokenCache);

// The off-runtime threads belong to the adapter: a canceled reload leaves its thread here, the semaphore bounds the live ones, and the drop joins them all.
pub struct Workers {
    slots: Arc<Semaphore>,
    live: Mutex<Vec<JoinHandle<()>>>,
}

impl Workers {
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(limit)),
            live: Mutex::new(Vec::new()),
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        let live = take(self.live.get_mut().unwrap_or_else(PoisonError::into_inner));
        for thread in live {
            drop(thread.join());
        }
    }
}

// A std thread carries no runtime handle, so a manager transport takes its blocking path; a panic drops the sender, which reads as unavailable.
pub(super) async fn off_runtime<Work, Output>(
    workers: &Workers,
    work: Work,
) -> Result<Output, OverlayError>
where
    Work: FnOnce() -> Output + Send + 'static,
    Output: Send + 'static,
{
    let slot = Arc::clone(&workers.slots)
        .acquire_owned()
        .await
        .map_err(|_closed| OverlayError::WorkerUnavailable)?;
    let (sender, answer) = oneshot::channel();
    let thread = thread::Builder::new()
        .name("config-overlay-worker".to_owned())
        .spawn(move || drop((slot, sender.send(work()))))
        .map_err(|source| OverlayError::WorkerStart { source })?;
    let mut live = workers.live.lock().unwrap_or_else(PoisonError::into_inner);
    live.retain(|thread| !thread.is_finished());
    live.push(thread);
    drop(live);
    answer
        .await
        .map_err(|_closed| OverlayError::WorkerUnavailable)
}

pub struct Resolver<'resolve> {
    pub lookup: &'resolve SecretLookup,
    pub env: &'resolve BTreeMap<String, String>,
}

impl Resolver<'_> {
    // _check_for_os_environ_vars: a mapping past the depth limit keeps its references, and a missing value becomes null
    pub fn resolve_entries<'value, Values>(
        &self,
        values: Values,
        depth: usize,
    ) -> Result<(), OverlayError>
    where
        Values: IntoIterator<Item = &'value mut JsonValue>,
    {
        if depth > DEFAULT_MAX_RECURSE_DEPTH {
            tracing::warn!(
                "Maximum recursion depth ({DEFAULT_MAX_RECURSE_DEPTH}) reached while processing config."
            );
            return Ok(());
        }
        for value in values {
            self.resolve_value(value, depth)?;
        }
        Ok(())
    }

    // _resolved_config_value: mappings recurse, mappings inside lists recurse, and other list items stay as written
    fn resolve_value(&self, value: &mut JsonValue, depth: usize) -> Result<(), OverlayError> {
        let nested = depth.saturating_add(1);
        match value {
            JsonValue::Object(fields) => self.resolve_entries(fields.values_mut(), nested),
            JsonValue::Array(items) => {
                let mappings = items.iter_mut().filter_map(JsonValue::as_object_mut);
                for fields in mappings {
                    self.resolve_entries(fields.values_mut(), nested)?;
                }
                Ok(())
            }
            JsonValue::String(text) if text.starts_with(ENVIRONMENT_PREFIX) => {
                let secret = self.lookup.get_secret(self.env, text, None)?;
                *value = match secret {
                    Some(SecretValue::Text(resolved)) => {
                        JsonValue::String(resolved.expose_secret().to_owned())
                    }
                    Some(SecretValue::Bool(flag)) => JsonValue::Bool(flag),
                    None => JsonValue::Null,
                };
                Ok(())
            }
            JsonValue::String(_) | JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {
                Ok(())
            }
        }
    }
}

// The worker thread owns what it reads, so this makes a lookup the reload can hand to it.
pub(super) fn owned_secret_lookup(lookup: &SecretLookup) -> SecretLookup {
    SecretLookup {
        manager: lookup.manager.as_ref().map(owned_manager),
        oidc_http: Arc::clone(&lookup.oidc_http),
        azure_tokens: Arc::clone(&lookup.azure_tokens),
        clock: Arc::clone(&lookup.clock),
        oidc_cache: Mutex::new(
            lookup
                .oidc_cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        ),
    }
}

// The tokens a resolution fetched outlive its worker, so they go back into the lookup the state keeps.
pub(super) fn restore_oidc_tokens(lookup: &SecretLookup, fetched: OidcTokenCache) {
    let mut cache = lookup
        .oidc_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    cache.extend(fetched.into_inner().unwrap_or_else(PoisonError::into_inner));
}

fn owned_manager(manager: &ManagerState) -> ManagerState {
    ManagerState {
        client: match &manager.client {
            Client::AzureKeyVault(client) => Client::AzureKeyVault(Arc::clone(client)),
            Client::GoogleKms(client) => Client::GoogleKms(Arc::clone(client)),
            Client::AwsKms(client) => Client::AwsKms(Arc::clone(client)),
            Client::AwsSecretManager(client) => Client::AwsSecretManager(Arc::clone(client)),
            Client::GoogleSecretManager(client) => Client::GoogleSecretManager(Arc::clone(client)),
            Client::Vault(client) => Client::Vault(Arc::clone(client)),
            Client::CyberArk(client) => Client::CyberArk(Arc::clone(client)),
            Client::Custom(client) => Client::Custom(Arc::clone(client)),
            Client::Infisical(client) => Client::Infisical(Arc::clone(client)),
            Client::Uninitialized => Client::Uninitialized,
        },
        settings: manager
            .settings
            .as_ref()
            .map(|settings| SecretManagerSettings {
                primary_secret_name: settings.primary_secret_name.clone(),
                optional_params: settings.optional_params.clone(),
            }),
        system: manager.system,
        access_mode: manager.access_mode,
        hosted_keys: manager.hosted_keys.clone(),
    }
}
