use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Mutex as TokioMutex;
use tokio_util::sync::CancellationToken;

use super::openapi::OpenApiHealthProbe;
use super::settings::HealthSettings;
use crate::registry::servers::SharedRegistry;

pub struct HealthDeps<P, S, E, A, C, L> {
    pub registry: SharedRegistry,
    pub probe: P,
    pub sessions: S,
    pub env_vars: E,
    pub allowed: A,
    pub clock: C,
    pub specs: L,
    pub cancel: CancellationToken,
}

pub struct HealthService<P, S, E, A, C, L> {
    pub(super) deps: HealthDeps<P, S, E, A, C, L>,
    pub(super) settings: HealthSettings,
    pub(super) spec_probes: Mutex<BTreeMap<String, Arc<TokioMutex<OpenApiHealthProbe>>>>,
    pub(super) instructions: Mutex<BTreeMap<String, String>>,
    pub(super) instructions_probed_at: Mutex<BTreeMap<String, Duration>>,
}

impl<P, S, E, A, C, L> HealthService<P, S, E, A, C, L> {
    #[must_use]
    pub fn new(deps: HealthDeps<P, S, E, A, C, L>, settings: HealthSettings) -> Self {
        Self {
            deps,
            settings,
            spec_probes: Mutex::new(BTreeMap::new()),
            instructions: Mutex::new(BTreeMap::new()),
            instructions_probed_at: Mutex::new(BTreeMap::new()),
        }
    }
}
