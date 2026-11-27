use std::sync::Arc;

use betterlite_auth::auth_key_lookup::KeyAuthenticator;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

pub struct Slot {
    pub authenticator: Option<Arc<dyn KeyAuthenticator>>,
}

impl Slot {
    #[must_use]
    pub const fn new(_config: &Config, _shared: &Shared) -> Self {
        // the key reader, auth cache and permission stores are composed by a later task
        Self {
            authenticator: None,
        }
    }
}
