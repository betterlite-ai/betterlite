use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::adapter::{Adapter, Environment};

const THINKING_KEYS: [&str; 3] = [
    "supports_adaptive_thinking",
    "thinking_always_on",
    "supports_legacy_thinking",
];

#[derive(Debug, Deserialize)]
pub(super) struct CatalogStep {
    model: String,
    custom_llm_provider: String,
}

impl CatalogStep {
    pub(super) async fn answer(
        &self,
        adapter: &Adapter<'_>,
        environment: &mut Environment<'_>,
    ) -> Value {
        let mut flags = Map::new();
        for key in THINKING_KEYS {
            let supported = supports_model_capability(
                &self.model,
                key,
                &self.custom_llm_provider,
                environment,
                &adapter.catalogs,
            )
            .await;
            flags.insert(key.to_owned(), Value::Bool(supported));
        }
        Value::Object(flags)
    }
}
