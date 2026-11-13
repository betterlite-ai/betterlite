use std::sync::Arc;

use betterlite_auth::auth_request_models::catalog::{EmptyCatalog, RequestModelResolver};
use betterlite_config::types::Config;

use crate::auth::auth_request_models::CoreDecoders;
use crate::state::shared::Shared;

pub struct Slot {
    resolver: Arc<RequestModelResolver>,
}

impl Slot {
    #[must_use]
    pub fn new(_config: &Config, _shared: &Shared) -> Self {
        Self {
            resolver: Arc::new(RequestModelResolver {
                catalog: Arc::new(EmptyCatalog),
                decoders: Arc::new(CoreDecoders),
            }),
        }
    }

    #[must_use]
    pub fn resolver(&self) -> Arc<RequestModelResolver> {
        Arc::clone(&self.resolver)
    }
}
