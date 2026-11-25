use super::swap::{SwapRequest, model_info_after_swap, resolve_swapped_provider};
use super::{MergeAnswer, MergeRequest, PromptError, PromptOutcome};
use crate::api::responses_parameters::prompt_merge::{
    ClientMessage, merge_prompt_management_input,
};
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::resolution::ModelResolver;
use betterlite_models::controls::json_value::JsonValue;
#[must_use]
pub(crate) fn text_value<'map>(kwargs: &'map ParameterMap, name: &str) -> Option<&'map str> {
    match kwargs.get(name) {
        Some(JsonValue::Text(text)) => Some(text),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}
pub(super) struct FinishRequest<'request> {
    pub(super) input: JsonValue,
    pub(super) model: &'request str,
    pub(super) custom_llm_provider: Option<&'request str>,
    pub(super) deployment_model_info: Option<JsonValue>,
    pub(super) answer: MergeAnswer,
    pub(super) kwargs: &'request ParameterMap,
    pub(super) local_vars: &'request mut ParameterMap,
    pub(super) resolver: &'request dyn ModelResolver,
}
pub(crate) fn merge_request<'request>(
    model: &'request str,
    messages: &'request [ClientMessage],
    kwargs: &'request ParameterMap,
) -> MergeRequest<'request> {
    MergeRequest {
        model,
        messages,
        non_default_params: kwargs,
        prompt_id: text_value(kwargs, "prompt_id"),
        prompt_variables: kwargs
            .get("prompt_variables")
            .filter(|value| **value != JsonValue::Null),
        prompt_label: text_value(kwargs, "prompt_label"),
        prompt_version: kwargs
            .get("prompt_version")
            .filter(|value| **value != JsonValue::Null),
        request_kwargs: kwargs,
    }
}
fn resolve_prompt_provider(
    model: &str,
    provider: Option<&str>,
    swapped_model: &str,
    kwargs: &ParameterMap,
    resolver: &dyn ModelResolver,
) -> Result<Option<String>, PromptError> {
    if swapped_model == model {
        return Ok(provider.map(str::to_owned));
    }
    resolve_swapped_provider(
        &SwapRequest {
            original_model: model,
            swapped_model,
            custom_llm_provider: provider,
            prompt_id: text_value(kwargs, "prompt_id"),
        },
        kwargs,
        resolver,
    )
    .map(Some)
}
pub(super) fn finish(request: FinishRequest<'_>) -> Result<PromptOutcome, PromptError> {
    let FinishRequest {
        input,
        model,
        custom_llm_provider,
        deployment_model_info,
        answer,
        kwargs,
        local_vars,
        resolver,
    } = request;
    let merged = merge_prompt_management_input(&input, &answer.messages)?;
    local_vars.insert(String::from("input"), merged.input.clone());
    local_vars.insert(String::from("model"), JsonValue::Text(answer.model.clone()));
    let provider =
        resolve_prompt_provider(model, custom_llm_provider, &answer.model, kwargs, resolver)?;
    let local = provider
        .as_deref()
        .map_or_else(|| JsonValue::Null, |name| JsonValue::Text(name.to_owned()));
    local_vars.insert(String::from("custom_llm_provider"), local);
    for (name, value) in answer.optional_params {
        local_vars.insert(name, value);
    }
    let model_info = model_info_after_swap(
        custom_llm_provider,
        provider.as_deref(),
        deployment_model_info,
    );
    Ok(PromptOutcome {
        input: merged.input,
        model: answer.model,
        custom_llm_provider: provider,
        deployment_model_info: model_info,
    })
}
