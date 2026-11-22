use crate::state::shared::Shared;
use betterlite_config::types::Config;

// LiteLLM reads general_settings on every check; the strict flag resolves once, when the state builds
pub struct Slot {
    pub fail_closed_budget_enforcement: bool,
}

impl Slot {
    #[must_use]
    pub const fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            fail_closed_budget_enforcement: config
                .sections
                .current_spend
                .fail_closed_budget_enforcement,
        }
    }
}
