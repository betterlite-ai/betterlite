use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    ThinkingRequest, is_effort_used, maybe_drop_disabled_thinking, translate_legacy_thinking,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};
use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
pub(super) struct ThinkingStep {
    model: String,
    custom_llm_provider: String,
    params: OptionalParams,
}

impl ThinkingStep {
    pub(super) async fn translate(
        &self,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Result<Value, Box<HelperError>> {
        let mut translated = self.params.clone();
        let request = ThinkingRequest {
            model: &self.model,
            custom_llm_provider: &self.custom_llm_provider,
            thresholds: adapter.thresholds,
        };
        match translate_legacy_thinking(&request, &mut translated, environment, &adapter.catalogs)
            .await
        {
            Ok(()) => serde_json::to_value(&translated).or_value_error(),
            Err(_error) => Ok(json!({"error": {"status": null}})),
        }
    }

    pub(super) async fn drop_disabled(
        &self,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Value {
        let mut dropped = self.params.clone();
        maybe_drop_disabled_thinking(
            &self.model,
            &mut dropped,
            &self.custom_llm_provider,
            environment,
            &adapter.catalogs,
        )
        .await;
        json!({"params": dropped, "warning": adapter.warnings.first()})
    }

    pub(super) async fn effort_used(
        &self,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Value {
        let used = is_effort_used(
            Some(&self.params),
            &self.model,
            &self.custom_llm_provider,
            environment,
            &adapter.catalogs,
        )
        .await;
        json!({"used": used})
    }
}
