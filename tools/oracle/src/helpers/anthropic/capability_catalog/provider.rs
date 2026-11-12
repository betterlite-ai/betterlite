use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use serde_json::{Map, Value};

use super::adapter::{Adapter, Environment};

const THINKING_KEYS: [&str; 3] = [
    "supports_adaptive_thinking",
    "thinking_always_on",
    "supports_legacy_thinking",
];

impl Adapter<'_> {
    pub(super) async fn provider_qualified_catalog(
        &self,
        model: &str,
        custom_llm_provider: &str,
        environment: &mut Environment<'_>,
    ) -> Value {
        let mut flags = Map::new();
        for key in THINKING_KEYS {
            let supported = supports_model_capability(
                model,
                key,
                custom_llm_provider,
                environment,
                &self.catalogs,
            )
            .await;
            flags.insert(key.to_owned(), Value::Bool(supported));
        }
        Value::Object(flags)
    }
}
