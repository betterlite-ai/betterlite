use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;

// The catalog a cycle reads: CatalogLookup's parts, with the provider hook shareable so the probe await can hold this
pub struct HealthCatalog<'catalog> {
    pub catalog: &'catalog DeploymentCatalog,
    pub model_group_alias: &'catalog AliasMap,
    pub routing_groups: &'catalog RoutingGroupIndex,
    pub inferred_provider: &'catalog (dyn Fn(&str) -> Option<String> + Send + Sync),
}

impl HealthCatalog<'_> {
    // B4442's view, rebuilt for one synchronous read so no borrowed catalog state ever crosses the probe await
    #[must_use]
    pub fn with_lookup<T, F>(&self, read: F) -> T
    where
        F: for<'read> FnOnce(CatalogLookup<'read>) -> T,
    {
        let inferred_provider = |model: &str| (self.inferred_provider)(model);
        let lookup = CatalogLookup {
            catalog: self.catalog,
            model_group_alias: self.model_group_alias,
            routing_groups: self.routing_groups,
            inferred_provider: &inferred_provider,
        };
        read(lookup)
    }
}
