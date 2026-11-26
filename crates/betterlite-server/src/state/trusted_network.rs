use std::sync::Arc;

use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// Pinned once at startup: the section already resolved every os.environ/ reference into typed values.
#[derive(Clone, Debug)]
pub struct Slot {
    pub settings: Arc<TrustedNetworkSettings>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            settings: Arc::new(TrustedNetworkSettings::from_config(
                &config.sections.trusted_network,
            )),
        }
    }
}
