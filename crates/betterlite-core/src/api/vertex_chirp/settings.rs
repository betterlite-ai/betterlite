use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;

use crate::api::api_boundaries::ApiLimits;

use super::ids::ChirpIds;

pub struct ChirpSettings {
    pub project: String,
    pub location: Option<String>,
    pub catalog: Arc<ModelCatalog>,
    pub limits: ApiLimits,
    pub ids: Box<dyn ChirpIds>,
}
