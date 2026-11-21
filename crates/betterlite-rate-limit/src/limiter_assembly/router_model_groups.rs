use std::sync::{Arc, PoisonError, RwLock};

use betterlite_config::sections::deployment_catalog::GroupAlias;
use indexmap::IndexMap;

use crate::limiter_records::ModelGroupResolver;

#[derive(Debug)]
pub struct RouterModelGroupResolver {
    // the router_settings.model_group_alias table, rewritten when saved router settings are applied
    pub aliases: Arc<RwLock<IndexMap<String, GroupAlias>>>,
}

impl ModelGroupResolver for RouterModelGroupResolver {
    fn resolve(&self, model: &str) -> Option<String> {
        let aliases = self.aliases.read().unwrap_or_else(PoisonError::into_inner);
        aliases
            .get(model)
            .map(|entry| entry.model.as_str())
            .filter(|target| !target.is_empty())
            .map(str::to_owned)
    }
}
