use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::pass_through_registry::PassThroughRegistry;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// what the virtual-key route stage reads: the JWT team routes B869 parses, and the pass-through registrations B8379 will install
pub struct Slot {
    pub team_allowed_routes: Vec<String>,
    pub registry: Arc<Mutex<PassThroughRegistry>>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        let registry = PassThroughRegistry {
            root_path: String::new(),
            registrations: Vec::new(),
        };
        Self {
            team_allowed_routes: config
                .sections
                .jwt_settings
                .routes
                .team_allowed_routes
                .clone(),
            registry: Arc::new(Mutex::new(registry)),
        }
    }

    #[must_use]
    pub fn registry_snapshot(&self) -> PassThroughRegistry {
        self.registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
