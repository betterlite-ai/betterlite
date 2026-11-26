use std::sync::Arc;

use betterlite_auth::auth_model_budgets::limits::{
    KeyModelBudgetLimiter, TeamModelBudgetLimiter, UserModelBudgetLimiter,
};
use betterlite_config::sections::auth_model_budgets::Settings;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

pub struct Slot {
    pub settings: Arc<Settings>,
    pub key_limiter: Option<Arc<dyn KeyModelBudgetLimiter>>,
    pub user_limiter: Option<Arc<dyn UserModelBudgetLimiter>>,
    pub team_limiter: Option<Arc<dyn TeamModelBudgetLimiter>>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            settings: Arc::new(config.sections.auth_model_budgets.clone()),
            key_limiter: None,
            user_limiter: None,
            team_limiter: None,
        }
    }
}
