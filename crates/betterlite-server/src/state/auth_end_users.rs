use crate::state::auth_resource_grants;
use crate::state::shared::Shared;
use betterlite_config::types::Config;

// LiteLLM's get_end_user_object runs only with a database, and the proxy default comes from litellm_settings
pub struct Slot {
    pub database: bool,
    pub max_end_user_budget_id: Option<String>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        Self {
            database: auth_resource_grants::Slot::new(config, shared).database,
            max_end_user_budget_id: config
                .sections
                .auth_end_users
                .max_end_user_budget_id
                .clone(),
        }
    }
}
