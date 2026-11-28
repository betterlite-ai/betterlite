use crate::state::shared::Shared;
use betterlite_config::types::Config;

// general_settings.admin_only_routes, which betterlite enforces without the license LiteLLM asks for
pub struct Slot {
    pub admin_only_routes: Vec<String>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            admin_only_routes: config
                .sections
                .auth_route_policy
                .admin_only_routes
                .clone()
                .unwrap_or_default(),
        }
    }
}
