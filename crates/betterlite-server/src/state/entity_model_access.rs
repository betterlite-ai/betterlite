use secrecy::ExposeSecret as _;

use crate::state::shared::Shared;
use betterlite_config::types::Config;

// LiteLLM reads teams, access groups and projects only when its prisma client exists
pub struct Slot {
    pub database: bool,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        let urls = &config.sections.database_urls;
        let config_url = urls.config_url.as_ref();
        let endpoint_url = urls.endpoints.database_url.as_ref();
        Self {
            database: config_url.is_some_and(|value| !value.expose_secret().is_empty())
                || endpoint_url.is_some_and(|value| !value.expose_secret().is_empty()),
        }
    }
}
