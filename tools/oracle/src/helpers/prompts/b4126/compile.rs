use std::collections::BTreeMap;

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptCompileRequest, PromptParams,
};
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::anthropic_cache_callbacks::hook::AnthropicCacheControlHook;
use betterlite_prompts::anthropic_cache_callbacks::result::compile_prompt_helper;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::orchestration::prompt_orchestration::PromptManagementBackend as _;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{HelperResult, OrHelperError as _};

use super::support::{self, Facts};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    prompt_id: Option<String>,
    #[serde(default)]
    prompt_spec: Option<Object>,
    #[serde(default)]
    prompt_variables: Option<PromptParams>,
    #[serde(default)]
    dynamic_callback_params: Option<ContractJson>,
    #[serde(default)]
    prompt_label: Option<String>,
    #[serde(default)]
    prompt_version: Option<i64>,
}

pub(super) fn empty(probes: &[Probe]) -> HelperResult {
    let mut clients = Vec::with_capacity(probes.len());
    for probe in probes {
        let spec = support::prompt_spec(probe.prompt_spec.as_ref())?;
        let dynamic = support::dynamic_params(probe.dynamic_callback_params.as_ref());
        let client = compile_prompt_helper(&request(probe, spec.as_ref(), &dynamic));
        clients.push(support::client_json(&client));
    }
    Ok(json!({ "clients": clients }))
}

pub(super) fn parity(probes: &[Probe]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| support::failure("RuntimeError", &error.to_string()))?;
    runtime.block_on(parity_async(probes))
}

async fn parity_async(probes: &[Probe]) -> HelperResult {
    let mut clients = Vec::with_capacity(probes.len());
    for probe in probes {
        let spec = support::prompt_spec(probe.prompt_spec.as_ref())?;
        let dynamic = support::dynamic_params(probe.dynamic_callback_params.as_ref());
        let hook = AnthropicCacheControlHook {
            environment: BTreeMap::new(),
            global_api_base: None,
            facts: Facts::default(),
        };
        let sync = compile_prompt_helper(&request(probe, spec.as_ref(), &dynamic));
        let async_request = PromptAsyncCompileRequest {
            prompt_id: probe.prompt_id.as_deref(),
            prompt_variables: probe.prompt_variables.as_ref(),
            dynamic_callback_params: &dynamic,
            prompt_spec: spec.as_ref(),
            prompt_label: probe.prompt_label.as_deref(),
            prompt_version: probe.prompt_version,
        };
        let asynchronous = hook
            .compile_template(&async_request)
            .await
            .or_value_error()?;
        clients.push(json!({
            "sync": support::client_json(&sync),
            "async": support::client_json(&asynchronous),
        }));
    }
    Ok(json!({ "clients": clients }))
}

fn request<'probe>(
    probe: &'probe Probe,
    spec: Option<&'probe PromptSpec>,
    dynamic: &'probe ContractJson,
) -> PromptCompileRequest<'probe> {
    PromptCompileRequest {
        prompt_id: probe.prompt_id.as_deref(),
        prompt_spec: spec,
        prompt_variables: probe.prompt_variables.as_ref(),
        dynamic_callback_params: dynamic,
        prompt_label: probe.prompt_label.as_deref(),
        prompt_version: probe.prompt_version,
    }
}
