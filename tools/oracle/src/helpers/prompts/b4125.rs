use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_defaults::CacheFacts;
use betterlite_prompts::cache_defaults::messages::{MessagesInjection, maybe_inject_cache_control};
use betterlite_prompts::cache_placement::system::SystemPrompt;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const BREAKPOINT_FLAG: &str = "supports_prompt_cache_breakpoint";
const PROMPT_CACHING_FLAG: &str = "supports_prompt_caching";
const PROVIDER_KEY: &str = "litellm_provider";
const PROVIDERS: [&str; 5] = ["anthropic", "azure_ai", "bedrock", "openai", "vertex_ai"];

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    MessagesInjection { probes: Vec<Probe> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    model: String,
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    params: Object,
    #[serde(default)]
    system: Option<SystemPrompt>,
    #[serde(default)]
    tools: Option<Vec<Json>>,
    #[serde(default)]
    global_api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    global_enable: bool,
    #[serde(default)]
    ttl: Option<String>,
    #[serde(default)]
    model_cost: BTreeMap<String, Json>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MessagesInjection { probes } => {
                let mut results = Vec::with_capacity(probes.len());
                for probe in probes {
                    results.push(result(probe)?);
                }
                Ok(json!({ "probes": results }))
            }
        }
    }
}

struct Facts<'input> {
    model_cost: &'input BTreeMap<String, Json>,
}

impl Facts<'_> {
    fn flag(&self, model: &str, name: &str) -> Option<bool> {
        match self.model_cost.get(model)?.as_object()?.get(name)? {
            Json::Bool(flag) => Some(*flag),
            Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

impl ModelFacts for Facts<'_> {
    fn cache_breakpoint_flag(&self, model: &str) -> Option<bool> {
        self.flag(model, BREAKPOINT_FLAG)
    }

    fn provider(&self, model: &str) -> Option<String> {
        if let Some((prefix, _rest)) = model.split_once('/')
            && PROVIDERS.contains(&prefix)
        {
            return Some(prefix.to_owned());
        }
        self.model_cost
            .get(model)?
            .as_object()?
            .get(PROVIDER_KEY)?
            .as_str()
            .map(str::to_owned)
    }
}

impl CacheFacts for Facts<'_> {
    fn supports_prompt_caching(&self, model: &str, _provider: &str) -> bool {
        let suffix = model.rsplit('/').next().unwrap_or(model);
        [model, suffix]
            .into_iter()
            .find_map(|key| self.flag(key, PROMPT_CACHING_FLAG))
            .unwrap_or(false)
    }
}

fn result(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let facts = Facts {
        model_cost: &probe.model_cost,
    };
    let mut params = probe.params.clone();
    let placed = maybe_inject_cache_control(
        MessagesInjection {
            messages: &probe.messages,
            system: probe.system.as_ref(),
            params: &mut params,
            model: Some(&probe.model),
            custom_llm_provider: None,
            tools: probe.tools.as_deref(),
            api_base: None,
            global_api_base: probe.global_api_base.as_deref(),
            environment: &probe.environment,
            policy: betterlite_prompts::cache_defaults::CachePolicy {
                global_enabled: probe.global_enable,
                ttl: probe.ttl.as_deref(),
            },
        },
        &facts,
    )
    .or_value_error()?;
    Ok(json!({
        "messages": placed.messages,
        "system": placed.system,
        "params": params,
    }))
}
