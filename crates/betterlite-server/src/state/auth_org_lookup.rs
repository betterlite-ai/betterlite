use betterlite_db::database_availability::policy::AllowRequestsOnOutage;

use crate::state::auth_resource_grants;
use crate::state::shared::Shared;
use betterlite_config::types::Config;

// get_org_object_for_request needs a database, DEFAULT_IN_MEMORY_TTL and the outage setting
pub struct Slot {
    pub database: bool,
    pub in_memory_ttl: f64,
    pub allow_requests_on_outage: AllowRequestsOnOutage,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        Self {
            database: auth_resource_grants::Slot::new(config, shared).database,
            in_memory_ttl: config.sections.auth_org_lookup.in_memory_ttl,
            allow_requests_on_outage: config.sections.database_availability.allow_requests.clone(),
        }
    }
}
