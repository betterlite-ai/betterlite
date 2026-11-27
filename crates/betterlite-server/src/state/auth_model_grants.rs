use std::sync::Arc;

use betterlite_auth::auth_model_grants::access_source::AgentAccessSource;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

pub struct Slot {
    // B8646 installs the registry-backed source here; B732 reads it when it builds the agent grant checks
    pub source: Option<Arc<dyn AgentAccessSource>>,
    pub enforce_fallback_model_access: bool,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            source: None,
            enforce_fallback_model_access: config
                .sections
                .auth_model_grants
                .enforce_fallback_model_access,
        }
    }
}
