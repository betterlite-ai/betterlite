use std::sync::Arc;

use betterlite_config::sections::deployment_catalog::GroupAlias;
use indexmap::IndexMap;

use crate::limiter_records::ModelGroupResolver;

#[derive(Debug)]
pub struct RouterModelGroupResolver {
    // the router_settings.model_group_alias table, which the router and this resolver both read
    pub aliases: Arc<IndexMap<String, GroupAlias>>,
}

impl ModelGroupResolver for RouterModelGroupResolver {
    fn resolve(&self, model: &str) -> Option<String> {
        self.aliases
            .get(model)
            .map(|entry| entry.model.as_str())
            .filter(|target| !target.is_empty())
            .map(str::to_owned)
    }
}
