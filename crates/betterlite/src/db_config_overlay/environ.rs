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
use tokio::sync::{Semaphore, oneshot, watch};

use super::error::OverlayError;

const ENVIRONMENT_PREFIX: &str = "os.environ/";
pub const WORKER_LIMIT: usize = 4;

pub type ResolvedConfig = (ConfigMap, OidcTokenCache);

// The off-runtime threads belong to the adapter: a canceled reload leaves its thread here, the semaphore bounds the live ones, and the stored drain lets every shutdown call join them.
pub struct Workers {
    pub slots: Arc<Semaphore>,
    pub live: Mutex<Vec<JoinHandle<()>>>,
    pub drain: Mutex<Option<watch::Receiver<bool>>>,
}

impl Workers {
    fn track(&self, thread: JoinHandle<()>) {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        live.retain(|handle| !handle.is_finished());
        live.push(thread);
    }

    // The server drains the live threads with this: the join runs on a thread of its own, so no runtime thread is blocked, and a canceled call leaves the drain stored for the next one.
    pub async fn shutdown(&self) -> Result<(), OverlayError> {
        let mut stored = self.drain.lock().unwrap_or_else(PoisonError::into_inner);
        if (*stored).as_ref().is_none_or(|done| *done.borrow()) {
            let live = take(&mut *self.live.lock().unwrap_or_else(PoisonError::into_inner));
            *stored = Some(Self::start_drain(live)?);
        }
        let mut drained = (*stored).clone().ok_or(OverlayError::WorkerUnavailable)?;
        drop(stored);
        while !*drained.borrow() {
            let changed = drained.changed().await;
            changed.map_err(|_closed| OverlayError::WorkerUnavailable)?;
        }
        Ok(())
    }

    // Every live handle joins here, and the drain it returns reports true once the last one is done.
    fn start_drain(live: Vec<JoinHandle<()>>) -> Result<watch::Receiver<bool>, OverlayError> {
        let (finished, running) = watch::channel(false);
        let _joiner = thread::Builder::new()
            .name("config-overlay-join".to_owned())
            .spawn(move || {
                for thread in live {
                    drop(thread.join());
                }
                let _sent = finished.send(true);
            })
            .map_err(|source| OverlayError::WorkerStart { source })?;
        Ok(running)
    }
}

// A std thread carries no runtime handle, so a manager transport takes its blocking path; a panic drops the sender, which reads as unavailable. Workers::track holds the live set's guard, so the future holds none across the await.
pub(super) async fn off_runtime<Work, Output>(
    workers: &Workers,
    work: Work,
) -> Result<Output, OverlayError>
where
    Work: FnOnce() -> Output + Send + 'static,
    Output: Send + 'static,
{
    let acquired = Arc::clone(&workers.slots).acquire_owned().await;
    let slot = acquired.map_err(|_closed| OverlayError::WorkerUnavailable)?;
    let (sender, answer) = oneshot::channel();
    let thread = thread::Builder::new()
        .name("config-overlay-worker".to_owned())
        .spawn(move || drop((slot, sender.send(work()))))
        .map_err(|source| OverlayError::WorkerStart { source })?;
    workers.track(thread);
    let outcome = answer.await;
    outcome.map_err(|_closed| OverlayError::WorkerUnavailable)
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
                for fields in items.iter_mut().filter_map(JsonValue::as_object_mut) {
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

pub(super) fn owned_secret_lookup(lookup: &SecretLookup) -> SecretLookup {
    let tokens = lookup
        .oidc_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    SecretLookup {
        manager: lookup.manager.as_ref().map(owned_manager),
        oidc_http: Arc::clone(&lookup.oidc_http),
        azure_tokens: Arc::clone(&lookup.azure_tokens),
        clock: Arc::clone(&lookup.clock),
        oidc_cache: Mutex::new(tokens),
    }
}

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
