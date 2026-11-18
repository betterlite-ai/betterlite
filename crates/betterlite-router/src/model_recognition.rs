pub mod kwargs;

mod tests;

use indexmap::IndexMap;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::model_catalog_lookup::alias::AliasMap;
use crate::routing_groups::group::RoutingGroup;

// The model list, aliases and Router._routing_groups groups one snapshot of the router serves.
pub struct ModelRecognition<'catalog> {
    pub catalog: &'catalog DeploymentCatalog,
    pub model_group_alias: &'catalog AliasMap,
    pub routing_groups: &'catalog IndexMap<String, RoutingGroup>,
}

impl ModelRecognition<'_> {
    // Router.get_routing_group: a deployment name or alias of this snapshot shadows a same-named group.
    #[must_use]
    pub fn routing_group(&self, model_name: &str) -> Option<&RoutingGroup> {
        if self.catalog.name_index.contains_key(model_name)
            || self.model_group_alias.contains_key(model_name)
        {
            return None;
        }
        self.routing_groups.get(model_name)
    }

    // Router.is_recognized_model: a deployment model_name, a deployment id, an alias, or a callable group.
    #[must_use]
    pub fn is_recognized(&self, model: &str) -> bool {
        self.catalog.model_names.contains(model)
            || self.catalog.id_index.contains_key(model)
            || self.model_group_alias.contains_key(model)
            || self.routing_group(model).is_some()
    }
}
