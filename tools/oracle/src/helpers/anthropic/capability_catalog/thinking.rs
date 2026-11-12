use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    ThinkingRequest, is_effort_used, maybe_drop_disabled_thinking, translate_legacy_thinking,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};
use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Clone, Copy)]
pub(super) enum ThinkingFunction {
    Translate,
    DropDisabled,
    EffortUsed,
}

#[derive(Debug, Deserialize)]
pub(super) struct ThinkingStep {
    model: String,
    custom_llm_provider: String,
    params: OptionalParams,
}

impl ThinkingStep {
    pub(super) async fn answer(
        &self,
        function: ThinkingFunction,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Result<Value, Box<HelperError>> {
        let model = self.model.as_str();
        let provider = self.custom_llm_provider.as_str();
        let catalogs = &adapter.catalogs;
        let mut params = self.params.clone();
        match function {
            ThinkingFunction::Translate => {
                let request = ThinkingRequest {
                    model,
                    custom_llm_provider: provider,
                    thresholds: adapter.thresholds,
                };
                match translate_legacy_thinking(&request, &mut params, environment, catalogs).await
                {
                    Ok(()) => serde_json::to_value(&params).or_value_error(),
                    Err(_error) => Ok(json!({"error": {"status": null}})),
                }
            }
            ThinkingFunction::DropDisabled => {
                maybe_drop_disabled_thinking(model, &mut params, provider, environment, catalogs)
                    .await;
                Ok(json!({"params": params, "warning": adapter.warnings.first()}))
            }
            ThinkingFunction::EffortUsed => {
                let used =
                    is_effort_used(Some(&params), model, provider, environment, catalogs).await;
                Ok(json!({"used": used}))
            }
        }
    }
}
