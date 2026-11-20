use std::sync::Arc;

use betterlite_auth::request_body_safety::{ModelGrantLookup, ModelGrants};
use betterlite_config::sections::request_body_safety::Settings;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// The pinned settings resolve once at startup; the Arc lets the auth stage carry them without copying, and the
// compatibility allowlist never authorizes a request destination in betterlite.
#[derive(Clone, Debug)]
pub struct Slot {
    pub settings: Arc<Settings>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            settings: Arc::new(config.sections.request_body_safety.clone()),
        }
    }
}

// get_model_group_info: no group answers Absent, a group without the option answers Unset.
impl ModelGrantLookup for Slot {
    fn grants(&self, model_group: &str) -> ModelGrants<'_> {
        match self.settings.model_grants.get(model_group) {
            None => ModelGrants::Absent,
            Some(None) => ModelGrants::Unset,
            Some(Some(items)) => ModelGrants::Granted(items),
        }
    }
}
