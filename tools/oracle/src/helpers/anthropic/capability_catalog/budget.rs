use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    EffortRequest, legacy_budget_to_effort,
};
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};

impl Adapter<'_> {
    pub(super) async fn legacy_budget_effort(
        &self,
        model: &str,
        custom_llm_provider: &str,
        budget_tokens: i64,
        environment: &mut Environment<'_>,
    ) -> Value {
        let request = EffortRequest {
            model,
            budget_tokens,
            custom_llm_provider,
            thresholds: self.thresholds,
        };
        let effort = legacy_budget_to_effort(&request, environment, &self.catalogs).await;
        json!({"effort": effort})
    }
}
