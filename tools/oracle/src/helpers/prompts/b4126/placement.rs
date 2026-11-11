use std::collections::BTreeMap;

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::{PromptIgnoreFlags, PromptParams};
use betterlite_prompts::anthropic_cache_callbacks::hook::AnthropicCacheControlHook;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::orchestration::prompt_orchestration::{
    ChatCompletionPromptRequest, PromptManagementBackend as _,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

use super::support::{self, Facts};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    model: String,
    #[serde(default)]
    messages: Vec<ContractJson>,
    #[serde(default)]
    params: PromptParams,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    global_api_base: Option<String>,
    #[serde(default)]
    model_cost: BTreeMap<String, Object>,
    #[serde(default)]
    prompt_id: Option<String>,
    #[serde(default)]
    prompt_variables: Option<PromptParams>,
    #[serde(default)]
    prompt_spec: Option<Object>,
    #[serde(default)]
    dynamic_callback_params: Option<ContractJson>,
    #[serde(default)]
    prompt_label: Option<String>,
    #[serde(default)]
    prompt_version: Option<i64>,
    #[serde(default)]
    ignore_model: bool,
    #[serde(default)]
    ignore_optional_params: bool,
}

pub(super) fn run(probes: &[Probe]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| support::failure("RuntimeError", &error.to_string()))?;
    runtime.block_on(execute(probes))
}

async fn execute(probes: &[Probe]) -> HelperResult {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        results.push(result(probe).await?);
    }
    Ok(json!({ "results": results }))
}

async fn result(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let spec = support::prompt_spec(probe.prompt_spec.as_ref())?;
    let dynamic = support::dynamic_params(probe.dynamic_callback_params.as_ref());
    let hook = AnthropicCacheControlHook {
        environment: probe.environment.clone(),
        global_api_base: probe.global_api_base.clone(),
        facts: Facts::from_model_cost(&probe.model_cost),
    };
    let request = ChatCompletionPromptRequest {
        model: &probe.model,
        messages: probe.messages.clone(),
        non_default_params: probe.params.clone(),
        prompt_id: probe.prompt_id.as_deref(),
        prompt_variables: probe.prompt_variables.as_ref(),
        dynamic_callback_params: &dynamic,
        prompt_spec: spec.as_ref(),
        prompt_label: probe.prompt_label.as_deref(),
        prompt_version: probe.prompt_version,
        ignore_flags: PromptIgnoreFlags {
            model: probe.ignore_model,
            optional_params: probe.ignore_optional_params,
        },
    };
    let placed = hook
        .async_completion_prompt(request)
        .await
        .or_value_error()?;
    Ok(json!({
        "model": placed.model,
        "messages": placed.messages,
        "params": placed.non_default_params,
    }))
}
