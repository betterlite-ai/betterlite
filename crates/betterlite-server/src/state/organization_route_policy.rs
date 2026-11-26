use crate::state::shared::Shared;
use betterlite_config::types::Config;

// the role checks read only the request body and its user, so this stage has no boot-time state
// the team-org lookup belongs to the route gate's caller (B867), which supplies the fetch itself
pub struct Slot;

impl Slot {
    #[must_use]
    pub const fn new(_config: &Config, _shared: &Shared) -> Self {
        Self
    }
}
