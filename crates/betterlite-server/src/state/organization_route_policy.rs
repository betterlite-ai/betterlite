use crate::state::shared::Shared;
use betterlite_config::types::Config;

pub struct Slot;

impl Slot {
    #[must_use]
    pub const fn new(_config: &Config, _shared: &Shared) -> Self {
        // the stage reads only the request and its user, so this slot holds no state
        Self
    }
}
