use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::cache_accounting::extraction::{request_system, request_user_agent};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    request_kwargs: Json,
    key: String,
}

pub(crate) fn extracts(probes: &[Probe]) -> Value {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        let value = probe
            .request_kwargs
            .as_object()
            .and_then(|fields| fields.get(&probe.key));
        let user_agent: Value = json!(request_user_agent(&probe.request_kwargs));
        results.push(json!({
            "value": value,
            "user_agent": user_agent,
            "system": request_system(&probe.request_kwargs),
        }));
    }
    json!({ "probes": results })
}
