use crate::catalog_updates::catalog::DeploymentCatalog;
use betterlite_openai::compatible_models::TokenLimits;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct DiscoveredDeploymentModelInfo {
    pub catalog: Arc<DeploymentCatalog>,
    pub index: usize,
    pub limits: TokenLimits,
}
impl DiscoveredDeploymentModelInfo {
    #[must_use]
    pub fn is_current(&self, catalog: &Arc<DeploymentCatalog>, model_id: &str) -> bool {
        match catalog.id_index.get(model_id) {
            Some(index) if Arc::ptr_eq(&self.catalog, catalog) => *index == self.index,
            None | Some(_) => false,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeploymentModelListingInfo {
    pub cost_map_keys: Vec<String>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
}
