use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::maybe_drop_disabled_thinking;
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};

impl Adapter<'_> {
    pub(super) async fn disabled_thinking_warning(
        &self,
        model: &str,
        custom_llm_provider: &str,
        params: &OptionalParams,
        environment: &mut Environment<'_>,
    ) -> Value {
        let mut dropped = params.clone();
        maybe_drop_disabled_thinking(
            model,
            &mut dropped,
            custom_llm_provider,
            environment,
            &self.catalogs,
        )
        .await;
        json!({
            "params": dropped,
            "warning": self.warnings.first(),
        })
    }
}
