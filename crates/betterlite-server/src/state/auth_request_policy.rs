use std::sync::Arc;

use betterlite_auth::request_body_safety::{ModelGrantLookup, ModelGrants};
use betterlite_config::sections::auth_request_policy::Settings;
use betterlite_config::types::Config;

use crate::state::request_body_safety::Slot as BodySafetySlot;
use crate::state::shared::Shared;

// Pinned once at startup; the body-safety slot answers the grants the ordered body check reads.
#[derive(Clone, Debug)]
pub struct Slot {
    pub settings: Arc<Settings>,
    pub body_safety: BodySafetySlot,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        Self {
            settings: Arc::new(config.sections.auth_request_policy.clone()),
            body_safety: BodySafetySlot::new(config, shared),
        }
    }
}

impl ModelGrantLookup for Slot {
    fn grants(&self, model_group: &str) -> ModelGrants<'_> {
        self.body_safety.grants(model_group)
    }
}
