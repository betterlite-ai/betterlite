use betterlite_auth::auth_context_sources::claude::RouterNames;
use betterlite_config::types::ModelDeployment;
use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_auth::auth_context_sources::sso::SsoConfiguration;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// The SSO settings resolve once at startup; readiness reads them and fetches nothing.
#[derive(Clone, Debug)]
pub struct Slot {
    pub sso: Arc<SsoConfiguration>,
    pub models: Vec<ModelDeployment>,
    pub aliases: BTreeSet<String>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            models: config.model_list.clone(),
            aliases: config
                .sections
                .deployment_catalog
                .model_group_alias
                .keys()
                .cloned()
                .collect(),
            sso: Arc::new(SsoConfiguration::from(
                &config.sections.auth_context_sources,
            )),
        }
    }
}

impl RouterNames for Slot {
    fn model_group_alias_contains(&self, name: &str) -> bool {
        self.aliases.contains(name)
    }
    fn has_model_id(&self, name: &str) -> bool {
        self.models
            .iter()
            .any(|model| model.model_info.id.as_deref() == Some(name))
    }
    fn candidate_model_ids_for_route(&self, name: &str, _team_id: Option<&str>) -> Vec<String> {
        self.models
            .iter()
            .filter(|model| model.model_name == name)
            .map(|model| {
                model
                    .model_info
                    .id
                    .clone()
                    .unwrap_or_else(|| model.model_name.clone())
            })
            .collect()
    }
}
