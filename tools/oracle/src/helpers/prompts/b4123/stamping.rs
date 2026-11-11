use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::cache_accounting::dialect::DialectInputs;
use betterlite_prompts::cache_accounting::points::CacheControlInjectionPoint;
use betterlite_prompts::cache_accounting::stamping::{
    stamped_for_prompt_hook, stamped_with_dialect,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::models::Facts;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    points: Vec<CacheControlInjectionPoint>,
    #[serde(default)]
    external_breakpoints: usize,
    model: String,
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

pub(crate) fn stamps(probes: &[Probe]) -> Value {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        results.push(probe_json(probe));
    }
    json!({ "probes": results })
}

fn probe_json(probe: &Probe) -> Value {
    let inputs = DialectInputs {
        model: &probe.model,
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
    json!({
        "for_prompt_hook": stamped_for_prompt_hook(
            &probe.points,
            probe.external_breakpoints,
            &inputs,
            &facts,
        ),
        "with_dialect": stamped_with_dialect(&probe.points, &inputs, &facts),
    })
}
