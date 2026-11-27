use crate::state::auth_resource_grants;
use crate::state::shared::Shared;
use betterlite_config::types::Config;

// LiteLLM walks the grants only when proxy_server.prisma_client exists
pub struct Slot {
    pub database: bool,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        Self {
            database: auth_resource_grants::Slot::new(config, shared).database,
        }
    }
}
