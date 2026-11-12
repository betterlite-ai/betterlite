use super::convert::convert;
use super::messages::{optional, take_flag};
use crate::completion_options::control::resolve_control_options;
use crate::completion_request::Preparation;
use crate::completion_request::args::JsonObject;
use crate::completion_request::call::PreparedCompletion;
use crate::completion_request::error::RequestResult;
use crate::completion_request::ports::{
    CacheSeedRequest, PromptOutcome, PromptRequest, ProviderLookup,
};
use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::filters::filter_out_litellm_params;
use betterlite_context::non_default_params::names::OPENAI_CHAT_COMPLETION_PARAMS;
use betterlite_models::controls::call_params::{DropParams, normalize_drop_params};
use betterlite_models::decisions::Json;
#[derive(Clone, Copy)]
pub(crate) enum HookForm {
    Blocking,
    Async,
}
pub(crate) fn flags(state: &mut Preparation<'_>) -> RequestResult<Option<PreparedCompletion>> {
    state.skip_bridge = take_flag(&mut state.kwargs, "_skip_responses_api_bridge")?;
    state.control =
        resolve_control_options(&state.kwargs, &state.call.model, state.settings.drop_params)?;
    let skip_mcp = take_flag(&mut state.kwargs, "_skip_mcp_handler")?;
    state.call.custom_llm_provider = optional::<String>(&state.kwargs, "custom_llm_provider")?;
    state.call.kwargs.clone_from(&state.kwargs);
    if skip_mcp || !super::finish::should_use_mcp_gateway(state.call.tools.as_deref()) {
        return Ok(None);
    }
    Ok(Some(PreparedCompletion::McpLoop(state.call.clone())))
}
pub(crate) fn non_default(state: &mut Preparation<'_>) -> RequestResult<()> {
    let names: Vec<&str> = OPENAI_CHAT_COMPLETION_PARAMS.split_whitespace().collect();
    let kwargs = convert::<Params, _>(&state.kwargs)?;
    state.non_default = convert::<JsonObject, _>(&filter_out_litellm_params(&kwargs, &names))?;
    Ok(())
}
pub(crate) fn cache_control(state: &mut Preparation<'_>) -> RequestResult<()> {
    let provider = optional::<String>(&state.kwargs, "custom_llm_provider")?;
    let given = optional::<String>(&state.kwargs, "api_base")?.filter(|value| !value.is_empty());
    let api_base = given.or_else(|| state.args.base_url.clone());
    let enable = optional::<bool>(&state.kwargs, "enable_prompt_caching")?;
    let request = CacheSeedRequest {
        messages: &state.call.messages,
        model: &state.call.model,
        custom_llm_provider: provider.as_deref(),
        tools: state.call.tools.as_deref(),
        enable_prompt_caching: enable,
        api_base: api_base.as_deref(),
    };
    state
        .ports
        .prompt
        .seed_cache_control(&request, &mut state.non_default);
    Ok(())
}
fn hook_request(state: &Preparation<'_>, form: HookForm) -> RequestResult<Option<PromptRequest>> {
    let prompt_id = optional::<String>(&state.kwargs, "prompt_id")?;
    let tools = match form {
        HookForm::Async => state.call.tools.as_deref(),
        HookForm::Blocking => None,
    };
    if !state
        .ports
        .prompt
        .should_run(prompt_id.as_deref(), &state.non_default, tools)
    {
        return Ok(None);
    }
    Ok(Some(PromptRequest {
        model: state.call.model.clone(),
        messages: state.call.messages.clone(),
        non_default_params: state.non_default.clone(),
        prompt_id,
        prompt_variables: optional::<JsonObject>(&state.kwargs, "prompt_variables")?,
        prompt_label: optional::<String>(&state.kwargs, "prompt_label")?,
        prompt_version: optional::<i64>(&state.kwargs, "prompt_version")?,
    }))
}
fn apply_prompt(state: &mut Preparation<'_>, outcome: PromptOutcome, form: HookForm) {
    state.call.model = outcome.model;
    state.call.messages = outcome.messages;
    state.non_default = outcome.non_default_params;
    if matches!(form, HookForm::Async) && state.call.tools.as_deref() == Some(&[]) {
        state.call.tools = None;
    }
}
pub(crate) async fn prompt_hook(state: &mut Preparation<'_>, form: HookForm) -> RequestResult<()> {
    let Some(request) = hook_request(state, form)? else {
        return Ok(());
    };
    let outcome = match form {
        HookForm::Blocking => {
            state
                .ports
                .prompt
                .get_chat_completion_prompt(request)
                .await?
        }
        HookForm::Async => state.ports.prompt.async_completion_prompt(request).await?,
    };
    apply_prompt(state, outcome, form);
    Ok(())
}
pub(crate) fn resolve_head_provider(state: &Preparation<'_>) -> RequestResult<String> {
    let given = optional::<String>(&state.kwargs, "api_base")?.filter(|value| !value.is_empty());
    let api_base = given.or_else(|| state.args.base_url.clone());
    let lookup = ProviderLookup {
        model: &state.call.model,
        custom_llm_provider: None,
        api_base: api_base.as_deref(),
        api_key: None,
    };
    let resolved = state.ports.model_metadata.resolve_provider(&lookup)?;
    Ok(resolved.provider.slug().to_owned())
}
fn is_router_call(kwargs: &JsonObject) -> bool {
    ["metadata", "litellm_metadata"].into_iter().any(|name| {
        matches!(kwargs.get(name), Some(Json::Object(bucket)) if bucket.contains_key("model_group"))
    })
}
pub(crate) fn router_params(state: &mut Preparation<'_>) -> RequestResult<()> {
    state.call.api_base = match &state.args.base_url {
        Some(base_url) => Some(base_url.clone()),
        None => optional::<String>(&state.kwargs, "api_base")?,
    };
    let num_retries = optional::<i64>(&state.kwargs, "num_retries")?;
    let max_retries = optional::<i64>(&state.kwargs, "max_retries")?;
    state.call.max_retries = if is_router_call(&state.kwargs) {
        Some(0)
    } else {
        num_retries.or(max_retries)
    };
    state.call.stream = state.args.stream.unwrap_or(false);
    state.call.choice_count = state.args.choice_count;
    Ok(())
}
pub(crate) fn drop_params_flag(kwargs: &JsonObject) -> Option<bool> {
    let encoded = serde_json::to_value(kwargs.get("drop_params")?).ok()?;
    let parsed: DropParams = serde_json::from_value(encoded).ok()?;
    normalize_drop_params(Some(&parsed))
}
pub(crate) fn provider_name(state: &Preparation<'_>) -> String {
    state
        .call
        .custom_llm_provider
        .clone()
        .unwrap_or_else(|| String::from("openai"))
}
