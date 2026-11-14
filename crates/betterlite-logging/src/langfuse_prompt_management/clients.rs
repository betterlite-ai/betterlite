use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use sha2::{Digest as _, Sha256};

use crate::langfuse_logger::{LangfuseLoggerError, LangfuseServices};
use crate::langfuse_rest::client::{LangfuseApiClient, LangfuseClientConfig};
use crate::langfuse_settings::{
    CredentialRequest, export, resolve_credentials, warn_if_upstream_configured,
};

use super::LangfusePromptManagementError;

// langfuse_prompt_management.langfuse_client_init's lru_cache(maxsize=10)
pub(crate) const PROMPT_CLIENT_CACHE_CAPACITY: usize = 10;

// the six langfuse_client_init arguments; every one of them is part of the cache key
#[derive(Clone, Copy, Debug)]
pub(crate) struct PromptClientRequest<'request> {
    pub(crate) public_key: Option<&'request str>,
    pub(crate) secret: Option<&'request str>,
    pub(crate) secret_key: Option<&'request str>,
    pub(crate) host: Option<&'request str>,
    pub(crate) flush_interval: u64,
    pub(crate) allow_env_credentials: bool,
}

impl<'request> PromptClientRequest<'request> {
    // a supplied host means the request brings its own deployment, so no environment key is mixed in
    pub(crate) fn from_dynamic_params(
        dynamic_params: &'request Json,
    ) -> Result<Self, LangfusePromptManagementError> {
        let Json::Object(fields) = dynamic_params else {
            return Err(LangfusePromptManagementError::ParamsType);
        };
        let public_key = credential_field(fields, "langfuse_public_key")?;
        let secret = credential_field(fields, "langfuse_secret")?;
        let secret_key = credential_field(fields, "langfuse_secret_key")?;
        let host = credential_field(fields, "langfuse_host")?;
        Ok(Self {
            public_key,
            secret,
            secret_key,
            host,
            flush_interval: super::PROMPT_MANAGER_FLUSH_INTERVAL,
            allow_env_credentials: host.is_none(),
        })
    }
}

fn credential_field<'request>(
    fields: &'request IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<&'request str>, LangfusePromptManagementError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.as_str())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(LangfusePromptManagementError::FieldType { field: name })
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ClientKey(String);

impl ClientKey {
    // the key is a digest because two of its six parts are secrets and only their equality matters
    fn new(request: &PromptClientRequest<'_>) -> Self {
        let mut hasher = Sha256::new();
        for part in [
            request.public_key,
            request.secret,
            request.secret_key,
            request.host,
        ] {
            match part {
                Some(value) => {
                    hasher.update([1_u8]);
                    hasher.update(value.as_bytes());
                    hasher.update([0_u8]);
                }
                None => hasher.update([2_u8]),
            }
        }
        hasher.update(request.flush_interval.to_le_bytes());
        hasher.update([u8::from(request.allow_env_credentials)]);
        Self(hex::encode(hasher.finalize()))
    }
}

struct CachedClient {
    used: u64,
    client: Arc<LangfuseApiClient>,
}

#[derive(Default)]
struct CacheState {
    tick: u64,
    entries: BTreeMap<ClientKey, CachedClient>,
}

// langfuse_client_init's lru_cache, bounded at ten clients per manager
#[derive(Default)]
pub(crate) struct LangfusePromptClients {
    state: Mutex<CacheState>,
}

impl LangfusePromptClients {
    // identical credentials and host hand back the client built first, like lru_cache
    pub(crate) fn client_init(
        &self,
        services: &LangfuseServices,
        request: &PromptClientRequest<'_>,
    ) -> Result<Arc<LangfuseApiClient>, LangfusePromptManagementError> {
        let key = ClientKey::new(request);
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let tick = next_tick(&mut state);
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.used = tick;
                return Ok(Arc::clone(&entry.client));
            }
        }
        let client = Arc::new(build_client(services, request)?);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let tick = next_tick(&mut state);
        if state.entries.len() >= PROMPT_CLIENT_CACHE_CAPACITY {
            evict_least_recently_used(&mut state);
        }
        state.entries.insert(
            key,
            CachedClient {
                used: tick,
                client: Arc::clone(&client),
            },
        );
        Ok(client)
    }
}

fn next_tick(state: &mut CacheState) -> u64 {
    state.tick = state.tick.saturating_add(1);
    state.tick
}

fn evict_least_recently_used(state: &mut CacheState) {
    let oldest = state
        .entries
        .iter()
        .min_by_key(|(_key, entry)| entry.used)
        .map(|(key, _entry)| key.clone());
    if let Some(oldest) = oldest {
        state.entries.remove(&oldest);
    }
}

// langfuse_client_init's checks and credential resolution before build_langfuse_client
fn build_client(
    services: &LangfuseServices,
    request: &PromptClientRequest<'_>,
) -> Result<LangfuseApiClient, LangfusePromptManagementError> {
    let settings = &services.settings;
    export::check_prompt_cache_ttl(settings).map_err(LangfuseLoggerError::Settings)?;
    let credentials = resolve_credentials(
        settings,
        CredentialRequest {
            public_key: request.public_key,
            secret: request.secret,
            secret_key: request.secret_key,
            host: request.host,
            allow_env_credentials: request.allow_env_credentials,
        },
    );
    warn_if_upstream_configured(settings);
    let timeout = export::timeout_seconds(settings)
        .map_err(LangfuseLoggerError::Settings)
        .and_then(|seconds| {
            Duration::try_from_secs_f64(seconds).map_err(|source| LangfuseLoggerError::Timeout {
                raw: settings.timeout.clone().unwrap_or_default(),
                source,
            })
        })?;
    let prompt_cache_ttl = Duration::try_from_secs_f64(export::prompt_cache_ttl_seconds(settings))
        .map_err(|source| LangfuseLoggerError::PromptCacheTtl { source })?;
    let config = LangfuseClientConfig {
        public_key: credentials.public_key,
        secret_key: credentials.secret_key,
        base_url: credentials.host,
        timeout,
        prompt_cache_ttl,
    };
    Ok(services.clients.build(config)?)
}
