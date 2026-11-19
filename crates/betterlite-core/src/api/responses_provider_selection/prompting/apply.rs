use betterlite_models::controls::json_value::JsonValue;

use super::swap::{SwapRequest, model_info_after_swap, resolve_swapped_provider};
use super::values::{present, text_value};
use super::{MergeAnswer, MergeRequest, PromptError, PromptOutcome};
use crate::api::responses_parameters::prompt_merge::{
    ClientMessage, merge_prompt_management_input,
};
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::resolution::ModelResolver;

pub(super) struct FinishRequest<'request> {
    pub(super) input: JsonValue,
    pub(super) model: &'request str,
    pub(super) custom_llm_provider: Option<&'request str>,
    pub(super) deployment_model_info: Option<JsonValue>,
    pub(super) answer: MergeAnswer,
    pub(super) kwargs: &'request ParameterMap,
    pub(super) local_vars: &'request mut ParameterMap,
}

pub(super) fn merge_request<'request>(
    model: &'request str,
    messages: &'request [ClientMessage],
    kwargs: &'request ParameterMap,
) -> MergeRequest<'request> {
    MergeRequest {
        model,
        messages,
        non_default_params: kwargs,
        prompt_id: text_value(kwargs, "prompt_id"),
        prompt_variables: present(kwargs, "prompt_variables"),
        prompt_label: text_value(kwargs, "prompt_label"),
        prompt_version: present(kwargs, "prompt_version"),
        request_kwargs: kwargs,
    }
}

pub(super) fn finish(
    request: FinishRequest<'_>,
    resolver: &dyn ModelResolver,
) -> Result<PromptOutcome, PromptError> {
    let FinishRequest {
        input,
        model,
        custom_llm_provider,
        deployment_model_info,
        answer,
        kwargs,
        local_vars,
    } = request;
    let merged = merge_prompt_management_input(&input, &answer.messages)?;
    local_vars.insert(String::from("input"), merged.input.clone());
    local_vars.insert(String::from("model"), JsonValue::Text(answer.model.clone()));
    let resolved_provider = if answer.model == model {
        custom_llm_provider.map(str::to_owned)
    } else {
        Some(resolve_swapped_provider(
            &SwapRequest {
                original_model: model,
                swapped_model: &answer.model,
                custom_llm_provider,
                prompt_id: text_value(kwargs, "prompt_id"),
            },
            kwargs,
            resolver,
        )?)
    };
    let local_provider = resolved_provider
        .as_ref()
        .map_or(JsonValue::Null, |provider| {
            JsonValue::Text(provider.clone())
        });
    local_vars.insert(String::from("custom_llm_provider"), local_provider);
    for (name, value) in answer.optional_params {
        local_vars.insert(name, value);
    }
    let deployment_model_info = model_info_after_swap(
        custom_llm_provider,
        resolved_provider.as_deref(),
        deployment_model_info,
    );
    Ok(PromptOutcome {
        input: merged.input,
        model: answer.model,
        custom_llm_provider: resolved_provider,
        deployment_model_info,
    })
}
