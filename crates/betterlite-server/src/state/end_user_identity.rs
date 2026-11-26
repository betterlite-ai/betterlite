use betterlite_config::sections::request_headers::Settings;
use betterlite_config::types::Config;

use crate::state::auth_resource_grants;
use crate::state::shared::Shared;

// B10050's extraction settings, the B1218 validation flag and the database the resolver needs
pub struct Slot {
    pub database: bool,
    pub validate_end_user_id_in_db: bool,
    pub max_end_user_budget_id: Option<String>,
    pub request_headers: Settings,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        let registry = &config.sections.end_user_registry;
        Self {
            database: auth_resource_grants::Slot::new(config, shared).database,
            validate_end_user_id_in_db: registry.validate_end_user_id_in_db,
            max_end_user_budget_id: registry.max_end_user_budget_id.clone(),
            request_headers: config.sections.request_headers.clone(),
        }
    }
}
