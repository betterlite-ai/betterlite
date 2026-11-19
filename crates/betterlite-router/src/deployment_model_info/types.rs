use std::sync::Arc;

use betterlite_openai::compatible_models::TokenLimits;

use crate::catalog_updates::catalog::DeploymentCatalog;

// types/router.py DiscoveredDeploymentModelInfo: the catalog generation and row a discovery belongs to, with the limits it advertised.
#[derive(Clone, Debug)]
pub struct DiscoveredDeploymentModelInfo {
    pub catalog: Arc<DeploymentCatalog>,
    pub index: usize,
    pub limits: TokenLimits,
}

impl DiscoveredDeploymentModelInfo {
    #[must_use]
    pub const fn new(catalog: Arc<DeploymentCatalog>, index: usize, limits: TokenLimits) -> Self {
        Self {
            catalog,
            index,
            limits,
        }
    }

    // get_discovered_model_info's identity test: the same generation, and the id still naming this record's row.
    #[must_use]
    pub fn is_current(&self, catalog: &Arc<DeploymentCatalog>, model_id: &str) -> bool {
        match catalog.id_index.get(model_id) {
            Some(index) if Arc::ptr_eq(&self.catalog, catalog) => *index == self.index,
            None | Some(_) => false,
        }
    }
}

// types/router.py DeploymentModelListingInfo: the cost-map keys a group's deployments name, and its widest configured limits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeploymentModelListingInfo {
    pub cost_map_keys: Vec<String>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
}
