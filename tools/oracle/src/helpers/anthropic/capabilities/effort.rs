use betterlite_anthropic::capabilities::effort::{EffortError, validate_effort_for_model};
use serde::Deserialize;
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};

#[derive(Debug, Deserialize)]
pub(super) struct ValidateStep {
    model: String,
    custom_llm_provider: String,
    effort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InvalidStep {
    model: String,
    llm_provider: String,
    value: String,
}

impl ValidateStep {
    pub(super) async fn answer(
        &self,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Value {
        validate_effort_for_model(
            &self.model,
            self.effort.as_deref(),
            &self.custom_llm_provider,
            environment,
            &adapter.catalogs,
        )
        .await
        .map_or(Value::Null, Value::String)
    }
}

impl InvalidStep {
    pub(super) fn answer(&self) -> Value {
        let error = EffortError::InvalidReasoningEffort {
            value: self.value.clone(),
            model: self.model.clone(),
            provider: self.llm_provider.clone(),
        };
        json!({"error": {"status": error.classify().metadata().status_code}})
    }
}
