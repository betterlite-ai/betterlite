pub mod cleanup;
pub mod error;
mod hydrate;
pub mod mutation;
pub mod owners;
pub mod ports;
mod prefix_owners;
mod registration;
pub mod reload;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use tokio_util::task::TaskTracker;

use self::ports::SharedToolOwners;
use crate::discovery_slots::DiscoverySlots;
use crate::registry::servers::SharedRegistry;
use crate::server_build::{BuildOptions, ServerBuilder};

const ENCRYPTED_DB_ENV_VARS: BuildOptions = BuildOptions {
    credentials_are_encrypted: true,
    env_vars_are_encrypted: Some(true),
};
const PLAIN_DB_ENV_VARS: BuildOptions = BuildOptions {
    credentials_are_encrypted: true,
    env_vars_are_encrypted: Some(false),
};

pub struct RegistryMutator<'collaborators, D, F, S, R, C, Rows, H, L, T, I, X, U> {
    pub builder: &'collaborators ServerBuilder<'collaborators, D, F, S, R>,
    pub registry: SharedRegistry,
    pub slots: Arc<DiscoverySlots<F, C>>,
    pub rows: &'collaborators Rows,
    pub tools: &'collaborators T,
    pub owners: SharedToolOwners,
    pub instructions: &'collaborators I,
    pub caches: &'collaborators X,
    pub dcr: &'collaborators H,
    pub lister: Arc<L>,
    pub tasks: TaskTracker,
    pub spec_fetcher: &'collaborators UserUrlFetcher<U>,
    pub client: &'collaborators ProxiedClient,
    pub warned_shadowed: WarnedIds,
    pub warned_capturing: WarnedIds,
}

// LiteLLM keeps each last-warned id set on the manager object; the mutex lets &self methods replace it
#[derive(Debug, Default)]
pub struct WarnedIds {
    ids: Mutex<BTreeSet<String>>,
}

impl WarnedIds {
    fn record(&self, current: &BTreeSet<String>) -> bool {
        let mut stored = self.ids.lock().unwrap_or_else(PoisonError::into_inner);
        let changed = !current.is_empty() && *stored != *current;
        stored.clone_from(current);
        changed
    }
}

mod tests;
