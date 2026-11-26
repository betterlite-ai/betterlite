use std::sync::Arc;

use betterlite_auth::auth_context_sources::sso::SsoConfiguration;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// The SSO settings resolve once at startup; readiness reads them and fetches nothing.
#[derive(Clone, Debug)]
pub struct Slot {
    pub sso: Arc<SsoConfiguration>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            sso: Arc::new(SsoConfiguration::from(
                &config.sections.auth_context_sources,
            )),
        }
    }
}
