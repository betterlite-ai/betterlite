use std::collections::BTreeMap;

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::PromptManagementClient;
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::anthropic_cache_callbacks::hook::AnthropicCacheControlHook;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::orchestration::prompt_orchestration::PromptManagementBackend as _;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const BREAKPOINT_FLAG: &str = "supports_prompt_cache_breakpoint";
const PROVIDER_KEY: &str = "litellm_provider";

#[derive(Default)]
pub(super) struct Facts {
    breakpoint_flags: BTreeMap<String, bool>,
    providers: BTreeMap<String, String>,
}

impl Facts {
    pub(super) fn from_model_cost(model_cost: &BTreeMap<String, Object>) -> Self {
        let mut facts = Self::default();
        for (model, entry) in model_cost {
            if let Some(Json::Bool(flag)) = entry.get(BREAKPOINT_FLAG) {
                facts.breakpoint_flags.insert(model.clone(), *flag);
            }
            if let Some(provider) = entry.get(PROVIDER_KEY).and_then(Json::as_str) {
                facts.providers.insert(model.clone(), provider.to_owned());
            }
        }
        facts
    }
}

impl ModelFacts for Facts {
    fn cache_breakpoint_flag(&self, model: &str) -> Option<bool> {
        self.breakpoint_flags.get(model).copied()
    }

    fn provider(&self, model: &str) -> Option<String> {
        self.providers.get(model).cloned()
    }
}

pub(super) fn failure(error_class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: error_class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

pub(super) fn prompt_spec(value: Option<&Object>) -> Result<Option<PromptSpec>, Box<HelperError>> {
    let Some(object) = value else {
        return Ok(None);
    };
    let encoded = serde_json::to_value(object).or_value_error()?;
    let spec = serde_json::from_value(encoded).or_value_error()?;
    Ok(Some(spec))
}

pub(super) fn dynamic_params(value: Option<&ContractJson>) -> ContractJson {
    value
        .cloned()
        .unwrap_or_else(|| ContractJson::Object(IndexMap::new()))
}

pub(super) fn client_json(client: &PromptManagementClient) -> Value {
    json!({
        "prompt_id": client.prompt_id,
        "messages": client.prompt_template,
        "model": client.prompt_template_model,
        "optional_params": client.prompt_optional_params,
        "completed_messages": client.completed_messages,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateProbe {
    #[serde(default)]
    prompt_id: Option<String>,
    #[serde(default)]
    prompt_spec: Option<Object>,
    #[serde(default)]
    dynamic_callback_params: Option<ContractJson>,
}

pub(super) fn integration_name() -> Value {
    let hook = AnthropicCacheControlHook {
        environment: BTreeMap::new(),
        global_api_base: None,
        facts: Facts::default(),
    };
    json!({ "name": hook.integration_name() })
}

pub(super) fn gate(probes: &[GateProbe]) -> HelperResult {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        let hook = AnthropicCacheControlHook {
            environment: BTreeMap::new(),
            global_api_base: None,
            facts: Facts::default(),
        };
        let spec = prompt_spec(probe.prompt_spec.as_ref())?;
        let dynamic = dynamic_params(probe.dynamic_callback_params.as_ref());
        results.push(hook.should_run(probe.prompt_id.as_deref(), spec.as_ref(), &dynamic));
    }
    Ok(json!({ "results": results }))
}
