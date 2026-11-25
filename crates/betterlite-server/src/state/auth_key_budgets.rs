use std::sync::Arc;

use betterlite_config::sections::auth_key_budgets::Settings;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

pub struct Slot {
    pub settings: Arc<Settings>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            settings: Arc::new(config.sections.auth_key_budgets.clone()),
        }
    }
}
