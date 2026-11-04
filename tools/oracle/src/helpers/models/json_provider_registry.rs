use betterlite_models::json_provider_registry::JSONProviderRegistry;
use serde_json::{Value, json};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let registry = match input.get("source").and_then(Value::as_str) {
            Some(source) => {
                let mut registry = JSONProviderRegistry::default();
                let bytes = source.as_bytes().to_vec();
                registry.load_from(|| Ok(bytes));
                registry
            }
            None => JSONProviderRegistry::load(),
        };
        let entries = registry
            .providers
            .values()
            .map(|entry| {
                json!({
                    "slug": entry.provider.to_string(),
                    "base_url": entry.base_url,
                    "api_key_env": entry.api_key_env,
                    "api_base_env": entry.api_base_env,
                    "base_class": entry.base_class,
                    "param_mappings": entry.param_mappings,
                    "constraints": entry.constraints,
                    "special_handling": entry.special_handling,
                    "supported_endpoints": entry.supported_endpoints,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({"entries": entries, "count": entries.len()}))
    }
}

mod tests;
