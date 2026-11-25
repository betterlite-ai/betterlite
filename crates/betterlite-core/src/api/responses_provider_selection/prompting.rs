mod apply;
pub mod swap;
use super::ParameterMap;
use super::prefixes::normalize_openai_chat_model;
use super::provider_config::{
    ASYNC_PROMPT_MERGED_PARAMS, ConfigLookup, bridges_to_chat_completions, resolve_provider_config,
};
use super::resolution::ModelResolver;
use crate::api::responses_parameters::error::ResponsesParametersError;
use crate::api::responses_parameters::prompt_merge::{
    ClientMessage, ReturnedMessage, input_to_chat_messages,
};
use apply::{FinishRequest, finish};
pub(crate) use apply::{merge_request, text_value};
use betterlite_models::controls::json_value::JsonValue;
use swap::PromptSwapError;
use thiserror::Error;
pub const CARRY_UNMATCHED_MESSAGE_POINTS: &str = "_litellm_carry_unmatched_cache_control_points";
#[derive(Debug, Error)]
#[error("the async prompt outcome is malformed: {0}")]
pub struct AsyncOutcomeError(pub String);
#[derive(Debug, Error)]
#[error("{message}")]
pub struct PromptManagerError {
    pub message: String,
}
pub struct HookRequest<'request> {
    pub prompt_id: Option<&'request str>,
    pub non_default_params: &'request ParameterMap,
}
pub struct MergeRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [ClientMessage],
    pub non_default_params: &'request ParameterMap,
    pub prompt_id: Option<&'request str>,
    pub prompt_variables: Option<&'request JsonValue>,
    pub prompt_label: Option<&'request str>,
    pub prompt_version: Option<&'request JsonValue>,
    pub request_kwargs: &'request ParameterMap,
}
#[derive(Debug, Clone)]
pub struct MergeAnswer {
    pub model: String,
    pub messages: Vec<ReturnedMessage>,
    pub optional_params: ParameterMap,
}
pub trait PromptManager {
    fn runs_prompt_hooks(&self, request: &HookRequest<'_>) -> bool;
    fn get_chat_completion_prompt(
        &self,
        request: &MergeRequest<'_>,
    ) -> Result<MergeAnswer, PromptManagerError>;
}
#[derive(Debug, Error)]
pub enum PromptError {
    #[error(transparent)]
    Merge(#[from] ResponsesParametersError),
    #[error(transparent)]
    Manager(#[from] PromptManagerError),
    #[error(transparent)]
    AsyncOutcome(#[from] AsyncOutcomeError),
    #[error(transparent)]
    Swap(#[from] PromptSwapError),
}
pub struct PromptRequest<'request> {
    pub input: JsonValue,
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub use_chat_completions_api: bool,
    pub kwargs: &'request mut ParameterMap,
    pub local_vars: &'request mut ParameterMap,
}
#[derive(Debug)]
pub struct PromptOutcome {
    pub input: JsonValue,
    pub model: String,
    pub custom_llm_provider: Option<String>,
    pub deployment_model_info: Option<JsonValue>,
}
struct HookCall<'request> {
    model: &'request str,
    custom_llm_provider: Option<&'request str>,
    use_chat_completions_api: bool,
}
pub struct ProvisionalBridge<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub use_chat_completions_api: bool,
    pub model_info: &'request JsonValue,
    pub api_base: Option<&'request str>,
}
#[must_use]
pub fn will_use_chat_bridge(lookup: &dyn ConfigLookup, bridge: &ProvisionalBridge<'_>) -> bool {
    let (model, prefixed) = normalize_openai_chat_model(bridge.model);
    let Some(provider) = bridge.custom_llm_provider else {
        return true;
    };
    let model_info = bridge.model_info;
    let api_base = bridge.api_base;
    let config = resolve_provider_config(lookup, &model, provider, model_info, api_base);
    bridges_to_chat_completions(config.as_ref(), bridge.use_chat_completions_api || prefixed)
}
#[must_use]
pub fn api_base_kwarg(kwargs: &ParameterMap) -> Option<&str> {
    let JsonValue::Text(base) = kwargs.get("api_base")? else {
        return None;
    };
    Some(base)
}
fn take_async_outcome(
    kwargs: &mut ParameterMap,
) -> Result<Option<(ParameterMap, Option<JsonValue>)>, AsyncOutcomeError> {
    let Some(value) = kwargs.shift_remove(ASYNC_PROMPT_MERGED_PARAMS) else {
        return Ok(None);
    };
    let JsonValue::Object(mut fields) = value else {
        return Err(AsyncOutcomeError(String::from("not an object")));
    };
    let Some(JsonValue::Object(merged)) = fields.shift_remove("merged_optional_params") else {
        return Err(AsyncOutcomeError(String::from("missing merged params")));
    };
    let deployment_model_info = match fields.shift_remove("deployment_model_info") {
        Some(JsonValue::Null) | None => None,
        Some(info) => Some(info),
    };
    Ok(Some((merged, deployment_model_info)))
}
fn untouched(
    input: JsonValue,
    model: &str,
    custom_llm_provider: Option<&str>,
    model_info: Option<JsonValue>,
) -> PromptOutcome {
    PromptOutcome {
        input,
        model: model.to_owned(),
        custom_llm_provider: custom_llm_provider.map(str::to_owned),
        deployment_model_info: model_info,
    }
}
pub fn apply_prompt_management(
    request: PromptRequest<'_>,
    manager: Option<&dyn PromptManager>,
    lookup: &dyn ConfigLookup,
    resolver: &dyn ModelResolver,
) -> Result<PromptOutcome, PromptError> {
    let PromptRequest {
        input,
        model,
        custom_llm_provider,
        use_chat_completions_api,
        kwargs,
        local_vars,
    } = request;
    if let Some((merged_optional_params, model_info)) = take_async_outcome(kwargs)? {
        local_vars.extend(merged_optional_params);
        return Ok(untouched(input, model, custom_llm_provider, model_info));
    }
    let client_input = input_to_chat_messages(Some(&input))?;
    let model_info = kwargs.get("model_info");
    let info = model_info.filter(|item| **item != JsonValue::Null).cloned();
    let hook = HookRequest {
        prompt_id: text_value(kwargs, "prompt_id"),
        non_default_params: kwargs,
    };
    let Some(found) = manager.filter(|found| found.runs_prompt_hooks(&hook)) else {
        return Ok(untouched(input, model, custom_llm_provider, info));
    };
    let call = HookCall {
        model,
        custom_llm_provider,
        use_chat_completions_api,
    };
    let Some(answer) = run_hook(found, &call, kwargs, &client_input, lookup)? else {
        return Ok(untouched(input, model, custom_llm_provider, info));
    };
    finish(FinishRequest {
        input,
        model,
        custom_llm_provider,
        deployment_model_info: info,
        answer,
        kwargs,
        local_vars,
        resolver,
    })
}
fn run_hook(
    manager: &dyn PromptManager,
    call: &HookCall<'_>,
    kwargs: &mut ParameterMap,
    client_input: &[ClientMessage],
    lookup: &dyn ConfigLookup,
) -> Result<Option<MergeAnswer>, PromptError> {
    let model_info = kwargs.get("model_info");
    let info = model_info.filter(|value| **value != JsonValue::Null);
    let bridge = ProvisionalBridge {
        model: call.model,
        custom_llm_provider: call.custom_llm_provider,
        use_chat_completions_api: call.use_chat_completions_api,
        model_info: info.unwrap_or(&JsonValue::Null),
        api_base: api_base_kwarg(kwargs),
    };
    let bridged = will_use_chat_bridge(lookup, &bridge);
    let carry = String::from(CARRY_UNMATCHED_MESSAGE_POINTS);
    if bridged {
        kwargs.insert(carry, JsonValue::Bool(true));
    }
    let answer =
        manager.get_chat_completion_prompt(&merge_request(call.model, client_input, kwargs));
    if bridged {
        kwargs.shift_remove(CARRY_UNMATCHED_MESSAGE_POINTS);
    }
    answer.map(Some).map_err(PromptError::from)
}
