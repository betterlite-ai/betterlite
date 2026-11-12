use betterlite_anthropic::capabilities::effort_param::model_supports_effort_param;
use serde::Deserialize;
use serde_json::Value;

use super::adapter::{Adapter, Environment};

#[derive(Debug, Deserialize)]
pub(super) struct EffortParamStep {
    model: String,
    custom_llm_provider: String,
}

impl EffortParamStep {
    pub(super) async fn answer(
        &self,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Value {
        Value::Bool(
            model_supports_effort_param(
                &self.model,
                &self.custom_llm_provider,
                environment,
                &adapter.catalogs,
            )
            .await,
        )
    }
}
