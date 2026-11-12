use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    EffortRequest, legacy_budget_to_effort,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};

#[derive(Debug, Deserialize)]
pub(super) struct BudgetStep {
    model: String,
    custom_llm_provider: String,
    budget_tokens: i64,
}

impl BudgetStep {
    pub(super) async fn answer(
        &self,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Value {
        let request = EffortRequest {
            model: &self.model,
            budget_tokens: self.budget_tokens,
            custom_llm_provider: &self.custom_llm_provider,
            thresholds: adapter.thresholds,
        };
        let effort = legacy_budget_to_effort(&request, environment, &adapter.catalogs).await;
        json!({"effort": effort})
    }
}
