use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::cache_accounting::dialect::{
    ModelFacts, gpt_version, model_map_breakpoint_flag, supports_openai_breakpoint,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapProbe {
    model: String,
    #[serde(default)]
    model_cost: BTreeMap<String, Json>,
}

pub(crate) struct Facts {
    pub(crate) model_cost: BTreeMap<String, Json>,
}

impl ModelFacts for Facts {
    fn cache_breakpoint_flag(&self, model: &str) -> Option<bool> {
        match self.model_cost.get(model)?.as_object()?.get(FLAG_KEY)? {
            Json::Bool(flag) => Some(*flag),
            Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
        }
    }

    fn provider(&self, model: &str) -> Option<String> {
        self.model_cost
            .get(model)
            .and_then(Json::as_object)
            .and_then(|entry| entry.get("litellm_provider"))
            .and_then(Json::as_str)
            .map(str::to_owned)
    }
}

const FLAG_KEY: &str = "supports_prompt_cache_breakpoint";

pub(crate) fn gpt_versions(models: &[String]) -> Value {
    let facts = Facts {
        model_cost: BTreeMap::new(),
    };
    let versions: Vec<Value> = models
        .iter()
        .map(|model| match gpt_version(model) {
            Some((major, minor)) => json!([major, minor]),
            None => Value::Null,
        })
        .collect();
    let supports: Vec<bool> = models
        .iter()
        .map(|model| supports_openai_breakpoint(model, &facts))
        .collect();
    json!({ "versions": versions, "supports": supports })
}

pub(crate) fn model_map(probes: &[MapProbe]) -> Value {
    let mut flags = Vec::with_capacity(probes.len());
    let mut supports = Vec::with_capacity(probes.len());
    for probe in probes {
        let facts = Facts {
            model_cost: probe.model_cost.clone(),
        };
        flags.push(json!(model_map_breakpoint_flag(&probe.model, &facts)));
        supports.push(json!(supports_openai_breakpoint(&probe.model, &facts)));
    }
    json!({ "flags": flags, "supports": supports })
}
