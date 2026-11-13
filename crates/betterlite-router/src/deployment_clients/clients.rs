use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use super::guard::{LimitSnapshot, LimitSource, MaxParallelRequestsLimit, installable_limit};

// RouterConfig.client_ttl: how long a cached client may serve before it is rebuilt
pub const CLIENT_TTL_SECONDS: u64 = 3600;

pub type ClientClock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ClientKind {
    Sync,
    Async,
}

impl ClientKind {
    // _get_client: the async keys are local-only in LiteLLM; every betterlite client is process-local
    #[must_use]
    pub fn cache_key(self, model_id: &str, stream: bool) -> String {
        match (self, stream) {
            (Self::Async, true) => format!("{model_id}_stream_async_client"),
            (Self::Async, false) => format!("{model_id}_async_client"),
            (Self::Sync, true) => format!("{model_id}_stream_client"),
            (Self::Sync, false) => format!("{model_id}_client"),
        }
    }
}

// Which credential a client was built from. A caller-supplied key never reuses the configured client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientCredential {
    Configured,
    // the SHA-256 hex of the request key, never the key itself
    Overridden(String),
}

// The identity B009's pool and B010's provider run with: the deployment's configured destination, never a request one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedClient {
    pub model_id: String,
    pub model_group: String,
    pub api_base: String,
    pub stream: bool,
    pub credential: ClientCredential,
}

#[derive(Clone, Debug, Default)]
pub struct ClientsSnapshot {
    pub limits: Vec<LimitSnapshot>,
    pub clients: Vec<CachedClient>,
}

struct Timed<T> {
    value: T,
    stored_at: SystemTime,
}

// Whether the cache held the limit and whether this lookup had to run the calculation at all.
#[derive(Clone, Debug)]
pub struct LimitLookup {
    pub limit: Option<Arc<MaxParallelRequestsLimit>>,
    pub initialized: bool,
}

pub struct CachedClients {
    ttl: Duration,
    clock: ClientClock,
    limits: Mutex<BTreeMap<String, Timed<Arc<MaxParallelRequestsLimit>>>>,
    clients: Mutex<BTreeMap<String, Timed<CachedClient>>>,
}

pub struct ClientLookup<'a> {
    pub model_id: &'a str,
    pub stream: bool,
    pub api_key: Option<&'a SecretString>,
}

pub struct ClientDeployment<'a> {
    pub model_group: &'a str,
    pub api_base: &'a str,
}

impl CachedClients {
    #[must_use]
    pub fn new(ttl: Duration, clock: ClientClock) -> Self {
        Self {
            ttl,
            clock,
            limits: Mutex::new(BTreeMap::new()),
            clients: Mutex::new(BTreeMap::new()),
        }
    }

    fn lock<T>(entries: &Mutex<T>) -> MutexGuard<'_, T> {
        entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn fresh<T>(ttl: Duration, now: SystemTime, entry: &Timed<T>) -> bool {
        now.duration_since(entry.stored_at)
            .is_ok_and(|age| age < ttl)
    }

    // The max_parallel_requests branch of _get_client: a cached limit, or one just calculated and cached.
    pub fn max_parallel_requests(
        &self,
        model_id: &str,
        model_group: &str,
        source: LimitSource,
    ) -> LimitLookup {
        let now = (self.clock)();
        let key = format!("{model_id}_max_parallel_requests_client");
        let mut limits = Self::lock(&self.limits);
        if let Some(entry) = limits.get(&key)
            && Self::fresh(self.ttl, now, entry)
        {
            return LimitLookup {
                limit: Some(Arc::clone(&entry.value)),
                initialized: false,
            };
        }
        limits.remove(&key);
        let limit = installable_limit(source, model_id, model_group);
        if let Some(installed) = &limit {
            limits.insert(
                key,
                Timed {
                    value: Arc::clone(installed),
                    stored_at: now,
                },
            );
        }
        LimitLookup {
            limit,
            initialized: true,
        }
    }

    // _get_async_openai_model_client: the configured client is reused while it is fresh, and a caller key that
    // differs from the cached one gets its own identity instead. The key never carries a request base URL.
    pub fn openai_client(
        &self,
        kind: ClientKind,
        lookup: &ClientLookup<'_>,
        deployment: &ClientDeployment<'_>,
    ) -> CachedClient {
        let credential = lookup.api_key.map_or(ClientCredential::Configured, |key| {
            ClientCredential::Overridden(hex::encode(Sha256::digest(
                key.expose_secret().as_bytes(),
            )))
        });
        let now = (self.clock)();
        let key = kind.cache_key(lookup.model_id, lookup.stream);
        let mut clients = Self::lock(&self.clients);
        if credential == ClientCredential::Configured {
            if let Some(entry) = clients.get(&key)
                && Self::fresh(self.ttl, now, entry)
                && entry.value.credential == ClientCredential::Configured
            {
                return entry.value.clone();
            }
            let configured = CachedClient {
                model_id: lookup.model_id.to_owned(),
                model_group: deployment.model_group.to_owned(),
                api_base: deployment.api_base.to_owned(),
                stream: lookup.stream,
                credential: ClientCredential::Configured,
            };
            clients.insert(
                key,
                Timed {
                    value: configured.clone(),
                    stored_at: now,
                },
            );
            return configured;
        }
        CachedClient {
            model_id: lookup.model_id.to_owned(),
            model_group: deployment.model_group.to_owned(),
            api_base: deployment.api_base.to_owned(),
            stream: lookup.stream,
            credential,
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> ClientsSnapshot {
        let limits = Self::lock(&self.limits);
        let clients = Self::lock(&self.clients);
        ClientsSnapshot {
            limits: limits
                .values()
                .map(|entry| entry.value.snapshot())
                .collect(),
            clients: clients.values().map(|entry| entry.value.clone()).collect(),
        }
    }
}
