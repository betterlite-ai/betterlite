pub mod async_outcome;
pub mod bridge;
pub mod swap;

mod apply;
mod values;

use betterlite_models::controls::json_value::JsonValue;
use thiserror::Error;

use super::ParameterMap;
use super::provider_config::ConfigLookup;
use super::resolution::ModelResolver;
use crate::api::responses_parameters::error::ResponsesParametersError;
use crate::api::responses_parameters::prompt_merge::{
    ClientMessage, ReturnedMessage, input_to_chat_messages,
};
use apply::{FinishRequest, finish, merge_request};
use async_outcome::AsyncOutcomeError;
use bridge::{ProvisionalBridge, api_base_kwarg, will_use_chat_bridge};
use swap::PromptSwapError;
use values::{present, text_value};

pub const CARRY_UNMATCHED_MESSAGE_POINTS: &str = "_litellm_carry_unmatched_cache_control_points";

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
    // LiteLLM declares prompt_version int | None, so the port receives the raw value
    pub prompt_version: Option<&'request JsonValue>,
    pub request_kwargs: &'request ParameterMap,
}

#[derive(Debug, Clone)]
pub struct MergeAnswer {
    pub model: String,
    pub messages: Vec<ReturnedMessage>,
    pub optional_params: ParameterMap,
}

// B8560 implements this over the prompt manager; the selection helpers only call these two methods
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
    kwargs: &'request mut ParameterMap,
}

// LiteLLM's _apply_prompt_management_to_responses_call: the merged input, model and provider plus kept deployment info
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
    if let Some(outcome) = async_outcome::take_async_outcome(kwargs)? {
        for (name, value) in outcome.merged_optional_params {
            local_vars.insert(name, value);
        }
        return Ok(PromptOutcome {
            input,
            model: model.to_owned(),
            custom_llm_provider: custom_llm_provider.map(str::to_owned),
            deployment_model_info: outcome.deployment_model_info,
        });
    }
    let client_input = input_to_chat_messages(Some(&input))?;
    let deployment_model_info = present(kwargs, "model_info").cloned();
    let Some(manager) = manager else {
        return Ok(unchanged(
            input,
            model,
            custom_llm_provider,
            deployment_model_info,
        ));
    };
    let hook = HookRequest {
        prompt_id: text_value(kwargs, "prompt_id"),
        non_default_params: kwargs,
    };
    if !manager.runs_prompt_hooks(&hook) {
        return Ok(unchanged(
            input,
            model,
            custom_llm_provider,
            deployment_model_info,
        ));
    }
    let call = HookCall {
        model,
        custom_llm_provider,
        use_chat_completions_api,
        kwargs,
    };
    let answer = run_hook(manager, call, &client_input, lookup)?;
    finish(
        FinishRequest {
            input,
            model,
            custom_llm_provider,
            deployment_model_info,
            answer,
            kwargs,
            local_vars,
        },
        resolver,
    )
}

fn unchanged(
    input: JsonValue,
    model: &str,
    custom_llm_provider: Option<&str>,
    deployment_model_info: Option<JsonValue>,
) -> PromptOutcome {
    PromptOutcome {
        input,
        model: model.to_owned(),
        custom_llm_provider: custom_llm_provider.map(str::to_owned),
        deployment_model_info,
    }
}

fn run_hook(
    manager: &dyn PromptManager,
    call: HookCall<'_>,
    client_input: &[ClientMessage],
    lookup: &dyn ConfigLookup,
) -> Result<MergeAnswer, PromptError> {
    let bridged = will_use_chat_bridge(
        lookup,
        &ProvisionalBridge {
            model: call.model,
            custom_llm_provider: call.custom_llm_provider,
            use_chat_completions_api: call.use_chat_completions_api,
            model_info: present(call.kwargs, "model_info").unwrap_or(&JsonValue::Null),
            api_base: api_base_kwarg(call.kwargs),
        },
    );
    if bridged {
        call.kwargs.insert(
            String::from(CARRY_UNMATCHED_MESSAGE_POINTS),
            JsonValue::Bool(true),
        );
    }
    let answer =
        manager.get_chat_completion_prompt(&merge_request(call.model, client_input, call.kwargs));
    if bridged {
        call.kwargs.shift_remove(CARRY_UNMATCHED_MESSAGE_POINTS);
    }
    answer.map_err(PromptError::from)
}
