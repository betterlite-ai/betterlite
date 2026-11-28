use std::sync::Arc;

use betterlite_auth::oauth_tokens::transport::TokenInfoTransport;
use betterlite_config::sections::oauth_tokens::Settings;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// Pinned once at startup: the section already resolved every os.environ/ reference into typed values.
pub struct Slot {
    pub settings: Arc<Settings>,
    pub transport: Arc<dyn TokenInfoTransport>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        Self {
            settings: Arc::new(config.sections.oauth_tokens.clone()),
            transport: Arc::new(shared.client.clone()),
        }
    }
}
