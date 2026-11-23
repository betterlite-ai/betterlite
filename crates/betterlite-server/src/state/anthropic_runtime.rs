use std::sync::Arc;

use betterlite_config::types::Config;
use betterlite_provider::anthropic_dispatch::gateway::runtime::{AnthropicRuntime, RuntimeError};

use crate::state::shared::Shared;

// the one Anthropic runtime a server builds; a configuration it cannot load fails here, at startup
pub struct Slot {
    pub runtime: Result<Arc<AnthropicRuntime>, Arc<RuntimeError>>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        let runtime =
            AnthropicRuntime::new(config, &shared.client, Arc::clone(&shared.bundled_catalog))
                .map(Arc::new)
                .map_err(Arc::new);
        Self { runtime }
    }
}
