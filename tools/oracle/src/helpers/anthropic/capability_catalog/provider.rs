use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};

impl Adapter<'_> {
    pub(super) async fn provider_qualified_catalog(
        &self,
        model: &str,
        custom_llm_provider: &str,
        environment: &mut Environment<'_>,
    ) -> Value {
        let adaptive = supports_model_capability(
            model,
            "supports_adaptive_thinking",
            custom_llm_provider,
            environment,
            &self.catalogs,
        )
        .await;
        let always = supports_model_capability(
            model,
            "thinking_always_on",
            custom_llm_provider,
            environment,
            &self.catalogs,
        )
        .await;
        let legacy = supports_model_capability(
            model,
            "supports_legacy_thinking",
            custom_llm_provider,
            environment,
            &self.catalogs,
        )
        .await;
        json!({
            "supports_adaptive_thinking": adaptive,
            "thinking_always_on": always,
            "supports_legacy_thinking": legacy,
        })
    }
}
