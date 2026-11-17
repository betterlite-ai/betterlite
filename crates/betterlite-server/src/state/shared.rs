use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;

pub struct Shared {
    pub bundled_catalog: Arc<ModelCatalog>,
}
