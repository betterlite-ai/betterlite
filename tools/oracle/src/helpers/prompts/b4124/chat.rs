use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_placement::chat::{ChatPrompt, apply_chat_injections};
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const FLAG_KEY: &str = "supports_prompt_cache_breakpoint";
const PROVIDER_KEY: &str = "litellm_provider";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    model: String,
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    params: Object,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    global_api_base: Option<String>,
    #[serde(default)]
    model_cost: BTreeMap<String, Json>,
}

struct Facts<'input> {
    model_cost: &'input BTreeMap<String, Json>,
}

impl ModelFacts for Facts<'_> {
    fn cache_breakpoint_flag(&self, model: &str) -> Option<bool> {
        match self.model_cost.get(model)?.as_object()?.get(FLAG_KEY)? {
            Json::Bool(flag) => Some(*flag),
            Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
        }
    }

    fn provider(&self, model: &str) -> Option<String> {
        self.model_cost
            .get(model)?
            .as_object()?
            .get(PROVIDER_KEY)?
            .as_str()
            .map(str::to_owned)
    }
}

pub(crate) fn run(probes: &[Probe]) -> HelperResult {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        results.push(result(probe)?);
    }
    Ok(json!({ "probes": results }))
}

fn result(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let facts = Facts {
        model_cost: &probe.model_cost,
    };
    let mut params = probe.params.clone();
    let processed = apply_chat_injections(ChatPrompt {
        model: &probe.model,
        messages: &probe.messages,
        params: &mut params,
        environment: &probe.environment,
        global_api_base: probe.global_api_base.as_deref(),
        facts: &facts,
    })
    .or_value_error()?;
    Ok(json!({ "messages": processed, "params": params }))
}
