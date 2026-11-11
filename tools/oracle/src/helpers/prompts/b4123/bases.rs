use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::cache_accounting::dialect::{
    DialectInputs, targets_openai_api, targets_openai_breakpoint,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::models::Facts;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    global_api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    prompt_cache_options: Option<Json>,
    #[serde(default)]
    model_cost: BTreeMap<String, Json>,
}

pub(crate) fn bases(probes: &[Probe]) -> Value {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        let inputs = DialectInputs {
            model: probe.model.as_deref().unwrap_or_default(),
            custom_llm_provider: probe.custom_llm_provider.as_deref(),
            api_base: probe.api_base.as_deref(),
            base_url: probe.base_url.as_deref(),
            global_api_base: probe.global_api_base.as_deref(),
            environment: &probe.environment,
            prompt_cache_options: probe.prompt_cache_options.as_ref(),
        };
        let facts = Facts {
            model_cost: probe.model_cost.clone(),
        };
        let dialects: Value = match probe.model {
            Some(_) => json!(targets_openai_breakpoint(&inputs, &facts)),
            None => Value::Null,
        };
        results.push(json!({
            "targets_api": targets_openai_api(&inputs),
            "targets_breakpoint": dialects,
        }));
    }
    json!({ "probes": results })
}
