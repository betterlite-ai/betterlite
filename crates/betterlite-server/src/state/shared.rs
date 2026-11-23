use std::sync::Arc;

use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;

pub struct Shared {
    pub bundled_catalog: Arc<ModelCatalog>,
    pub client: ProxiedClient,
}
