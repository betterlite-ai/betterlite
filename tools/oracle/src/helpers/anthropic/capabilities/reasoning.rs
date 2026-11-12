use betterlite_anthropic::capabilities::reasoning::{ReasoningRequest, map_reasoning_effort};
use serde::Deserialize;
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};
use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
pub(super) struct MapStep {
    model: String,
    custom_llm_provider: String,
    llm_provider: String,
    reasoning_effort: Option<String>,
}

impl MapStep {
    pub(super) async fn answer(
        &self,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Result<Value, Box<HelperError>> {
        let request = ReasoningRequest {
            reasoning_effort: self.reasoning_effort.as_deref(),
            model: &self.model,
            custom_llm_provider: &self.custom_llm_provider,
            llm_provider: &self.llm_provider,
            budgets: &adapter.budgets,
        };
        match map_reasoning_effort(&request, environment, &adapter.catalogs).await {
            Ok(Some(thinking)) => serde_json::to_value(thinking).or_value_error(),
            Ok(None) => Ok(Value::Null),
            Err(error) => Ok(json!({"error": {"status": error.classify().metadata().status_code}})),
        }
    }
}
