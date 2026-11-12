use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    ThinkingRequest, translate_legacy_thinking,
};
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};
use crate::helpers::{HelperError, OrHelperError as _};

impl Adapter<'_> {
    pub(super) async fn adaptive_thinking(
        &self,
        model: &str,
        custom_llm_provider: &str,
        params: &OptionalParams,
        environment: &mut Environment<'_>,
    ) -> Result<Value, Box<HelperError>> {
        let mut translated = params.clone();
        let request = ThinkingRequest {
            model,
            custom_llm_provider,
            thresholds: self.thresholds,
        };
        match translate_legacy_thinking(&request, &mut translated, environment, &self.catalogs)
            .await
        {
            Ok(()) => serde_json::to_value(&translated).or_value_error(),
            Err(_error) => Ok(json!({"error": {"status": null}})),
        }
    }
}
