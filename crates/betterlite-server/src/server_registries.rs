use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_vertex::credential_sources::sdk::CredentialClock;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_cache::executor::CredentialExecutor;

use crate::pass_through_credentials::{PassthroughEndpointRouter, ProviderResolver};
use crate::passthrough::route_registration::{PassThroughRoutes, SharedPassThroughRoutes};

pub mod mcp;
pub mod vectors;

mod tests;

pub struct PassThroughState {
    pub credentials: Arc<RwLock<PassthroughEndpointRouter>>,
    pub vertex_tokens: Arc<CredentialCache>,
    pub routes: SharedPassThroughRoutes,
}

impl PassThroughState {
    #[must_use]
    pub fn new(
        clock: Arc<dyn CredentialClock>,
        executor: Arc<dyn CredentialExecutor>,
        resolve_provider: ProviderResolver,
        credentials: CredentialSnapshot,
        environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    ) -> Self {
        Self {
            credentials: Arc::new(RwLock::new(PassthroughEndpointRouter {
                deployments: Vec::new(),
                resolve_provider,
                credentials,
                environment,
                vertex_by_deployment_key: BTreeMap::new(),
                default_vertex_config: None,
            })),
            vertex_tokens: Arc::new(CredentialCache::new(clock, executor)),
            routes: Arc::new(Mutex::new(PassThroughRoutes::default())),
        }
    }
}
