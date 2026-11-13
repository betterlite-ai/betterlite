pub mod prompts;

use crate::state::shared::Shared;
use betterlite_config::types::Config;

pub struct Slot;

impl Slot {
    #[must_use]
    pub const fn new(_config: &Config, _shared: &Shared) -> Self {
        // the callback logger owner fills this slot
        Self
    }
}
