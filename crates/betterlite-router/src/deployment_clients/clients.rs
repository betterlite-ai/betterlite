use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use super::guard::{LimitSource, MaxParallelRequestsLimit, installable_limit};

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

struct Timed<T> {
    value: T,
    stored_at: SystemTime,
}

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

pub struct ClientLookup<'lookup> {
    pub model_id: &'lookup str,
    pub stream: bool,
    pub api_key: Option<&'lookup SecretString>,
}

pub struct ClientDeployment<'deployment> {
    pub model_group: &'deployment str,
    pub api_base: &'deployment str,
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

    fn fresh<T>(ttl: Duration, now: SystemTime, entry: &Timed<T>) -> bool {
        now.duration_since(entry.stored_at)
            .is_ok_and(|age| age < ttl)
    }

    // the max_parallel_requests branch of the client lookup: a cached limit, or one calculated and cached now
    pub fn max_parallel_requests(
        &self,
        model_id: &str,
        model_group: &str,
        source: LimitSource,
    ) -> LimitLookup {
        let now = (self.clock)();
        let key = format!("{model_id}_max_parallel_requests_client");
        let mut limits = self.limits.lock().unwrap_or_else(PoisonError::into_inner);
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
        drop(limits);
        LimitLookup {
            limit,
            initialized: true,
        }
    }

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
        let client = CachedClient {
            model_id: lookup.model_id.to_owned(),
            model_group: deployment.model_group.to_owned(),
            api_base: deployment.api_base.to_owned(),
            stream: lookup.stream,
            credential,
        };
        if client.credential != ClientCredential::Configured {
            return client;
        }
        let now = (self.clock)();
        let key = kind.cache_key(lookup.model_id, lookup.stream);
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = clients.get(&key)
            && Self::fresh(self.ttl, now, entry)
        {
            return entry.value.clone();
        }
        clients.insert(
            key,
            Timed {
                value: client.clone(),
                stored_at: now,
            },
        );
        client
    }
}
