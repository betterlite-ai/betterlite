use betterlite_config::types::Config;

use crate::state::shared::Shared;

pub struct Slot {
    pub max_recurse_depth: i64,
}

impl Slot {
    #[must_use]
    pub const fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            max_recurse_depth: config.sections.database_reencryption.max_recurse_depth,
        }
    }
}
