use std::sync::Arc;

use crate::route_read_through::ModelReadThrough;
use crate::state::shared::Shared;
use betterlite_config::types::Config;

pub struct Slot {
    pub read_through: Option<Arc<dyn ModelReadThrough>>,
}

impl Slot {
    #[must_use]
    pub const fn new(_config: &Config, _shared: &Shared) -> Self {
        // the production read-through reaches Shared in a later task
        Self { read_through: None }
    }
}
