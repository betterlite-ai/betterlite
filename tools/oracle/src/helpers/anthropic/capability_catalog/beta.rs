use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::is_effort_used;
use serde_json::{Value, json};

use super::adapter::{Adapter, Environment};

impl Adapter<'_> {
    pub(super) async fn effort_beta_gate(
        &self,
        model: &str,
        custom_llm_provider: &str,
        params: &OptionalParams,
        environment: &mut Environment<'_>,
    ) -> Value {
        let used = is_effort_used(
            Some(params),
            model,
            custom_llm_provider,
            environment,
            &self.catalogs,
        )
        .await;
        json!({"used": used})
    }
}
