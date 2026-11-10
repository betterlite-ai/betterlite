mod tests;

use std::time::Duration;

use super::budget_config::BudgetRuntime;
use super::budget_scope::BudgetScope;
use super::resolution::ResolvedModelBudget;
use super::spend_keys::model_budget_spend_cache_key;

#[derive(Clone, Copy)]
pub struct LegacyCarry<'parser> {
    pub runtime: BudgetRuntime<'parser>,
    pub uptime: Duration,
}

impl LegacyCarry<'_> {
    #[must_use]
    pub fn request_model_spend_cache_key(
        &self,
        scope: BudgetScope,
        entity_id: &str,
        request_model: &str,
        resolved: &ResolvedModelBudget,
    ) -> Option<String> {
        if !matches!(scope, BudgetScope::Key | BudgetScope::EndUser) {
            return None;
        }
        let budget_duration = &resolved.budget_config.budget_duration;
        let window_seconds = self
            .runtime
            .durations
            .seconds(budget_duration, self.runtime.now)?;
        let Ok(window_secs) = u64::try_from(window_seconds) else {
            return None;
        };
        if self.uptime >= Duration::from_secs(window_secs) {
            return None;
        }
        let legacy = model_budget_spend_cache_key(scope, entity_id, request_model, budget_duration);
        let current =
            model_budget_spend_cache_key(scope, entity_id, &resolved.budget_model, budget_duration);
        (legacy != current).then_some(legacy)
    }
}
