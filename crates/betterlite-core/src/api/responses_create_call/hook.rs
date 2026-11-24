use indexmap::IndexMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::response_input::ResponsesInput;

use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::error::CreateError;
use crate::api::responses_create::preparation::connection_kwargs;
use crate::api::responses_parameters::prompt_merge::{
    ClientMessage, input_to_chat_messages, merge_prompt_management_input,
};
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prefixes::pop_use_chat_completions_api;
use crate::api::responses_provider_selection::prompting::swap::{
    PromptSwapError, SwapRequest, model_info_after_swap, resolve_swapped_provider,
};
use crate::api::responses_provider_selection::prompting::{
    CARRY_UNMATCHED_MESSAGE_POINTS, HookRequest, MergeAnswer, PromptError, PromptManagerError,
    ProvisionalBridge, api_base_kwarg, merge_request, text_value, will_use_chat_bridge,
};
use crate::api::responses_provider_selection::provider_config::{
    ASYNC_PROMPT_MERGED_PARAMS, ConfigLookup,
};
use crate::api::responses_provider_selection::resolution::ModelResolver;

use super::deps::{AsyncCreateDeps, AsyncPromptPort};
use super::error::{AsyncCreateCause, AsyncCreateError};
use super::prepare::PreparedCall;

pub(super) async fn prompted(
    deps: &AsyncCreateDeps<'_>,
    call: &ResponsesCreateCall,
    converted: ResponsesCreateCall,
    provider: String,
    port: &dyn AsyncPromptPort,
) -> Result<PreparedCall, AsyncCreateError> {
    let mut kwargs = connection_kwargs(&converted);
    let hook = HookRequest {
        prompt_id: text_value(&kwargs, "prompt_id"),
        non_default_params: &kwargs,
    };
    if !port.runs_prompt_hooks(&hook) {
        return Ok(PreparedCall {
            call: converted,
            provider,
            model: call.model.clone(),
        });
    }
    let client_input = client_messages(call, &provider)?;
    let bridged = bridges(deps.lookup, call, &provider, &kwargs);
    let answer = match run_hook(port, call, &mut kwargs, &client_input, bridged).await {
        Ok(answer) => answer,
        Err(error) => {
            return Err(AsyncCreateError {
                cause: Box::new(AsyncCreateCause::Create(CreateError::Prompt(
                    PromptError::Manager(error),
                ))),
                provider: provider.clone(),
                model: call.model.clone(),
            });
        }
    };
    let state = Answer {
        call,
        provider,
        kwargs,
        merged: answer,
    };
    let input = state.input()?;
    let final_provider = state.provider(deps.resolver)?;
    Ok(state.outcome(input, final_provider, converted))
}

// The client's input as chat messages, read the way the sync entry reads it before the hook replaces it
fn client_messages(
    call: &ResponsesCreateCall,
    provider: &str,
) -> Result<Vec<ClientMessage>, AsyncCreateError> {
    let original = transcode(&call.input).map_err(|error| AsyncCreateError {
        cause: Box::new(AsyncCreateCause::Create(CreateError::from(error))),
        provider: provider.to_owned(),
        model: call.model.clone(),
    })?;
    input_to_chat_messages(Some(&original)).map_err(|error| AsyncCreateError {
        cause: Box::new(AsyncCreateCause::Create(CreateError::Prompt(
            PromptError::Merge(error),
        ))),
        provider: provider.to_owned(),
        model: call.model.clone(),
    })
}

// LiteLLM reads the flag without consuming it, and the sync call still pops it for itself
fn bridges(
    lookup: &dyn ConfigLookup,
    call: &ResponsesCreateCall,
    provider: &str,
    kwargs: &ParameterMap,
) -> bool {
    let mut probe = kwargs.clone();
    will_use_chat_bridge(
        lookup,
        &ProvisionalBridge {
            model: &call.model,
            custom_llm_provider: Some(provider),
            use_chat_completions_api: pop_use_chat_completions_api(&mut probe),
            model_info: kwargs.get("model_info").unwrap_or(&JsonValue::Null),
            api_base: api_base_kwarg(kwargs),
        },
    )
}

// LiteLLM holds the carry signal only while the hook that sees the provisional message list runs
pub(super) async fn run_hook(
    port: &dyn AsyncPromptPort,
    call: &ResponsesCreateCall,
    kwargs: &mut ParameterMap,
    client_input: &[ClientMessage],
    bridged: bool,
) -> Result<MergeAnswer, PromptManagerError> {
    if bridged {
        kwargs.insert(
            String::from(CARRY_UNMATCHED_MESSAGE_POINTS),
            JsonValue::Bool(true),
        );
    }
    let request = merge_request(&call.model, client_input, kwargs);
    let answer = port.async_merge_prompt(&request).await;
    if bridged {
        kwargs.shift_remove(CARRY_UNMATCHED_MESSAGE_POINTS);
    }
    answer
}

struct Answer<'call> {
    call: &'call ResponsesCreateCall,
    provider: String,
    kwargs: ParameterMap,
    merged: MergeAnswer,
}

impl Answer<'_> {
    fn input(&self) -> Result<ResponsesInput, AsyncCreateError> {
        let original = transcode(&self.call.input).map_err(|error| AsyncCreateError {
            cause: Box::new(AsyncCreateCause::Create(CreateError::from(error))),
            provider: self.provider.clone(),
            model: self.merged.model.clone(),
        })?;
        let merged = match merge_prompt_management_input(&original, &self.merged.messages) {
            Ok(outcome) => outcome.input,
            Err(error) => {
                return Err(AsyncCreateError {
                    cause: Box::new(AsyncCreateCause::Create(CreateError::Prompt(
                        PromptError::Merge(error),
                    ))),
                    provider: self.provider.clone(),
                    model: self.merged.model.clone(),
                });
            }
        };
        transcode(&merged).map_err(|error| AsyncCreateError {
            cause: Box::new(AsyncCreateCause::Create(CreateError::Rewrite(error))),
            provider: self.provider.clone(),
            model: self.merged.model.clone(),
        })
    }

    fn provider(&self, resolver: &dyn ModelResolver) -> Result<String, AsyncCreateError> {
        if self.merged.model == self.call.model {
            return Ok(self.provider.clone());
        }
        swapped_provider(
            resolver,
            self.call,
            &self.provider,
            &self.merged.model,
            &self.kwargs,
        )
    }

    fn outcome(
        self,
        input: ResponsesInput,
        final_provider: String,
        mut converted: ResponsesCreateCall,
    ) -> PreparedCall {
        let info = self.kwargs.get("model_info").cloned();
        let deployment = model_info_after_swap(
            Some(self.provider.as_str()),
            Some(final_provider.as_str()),
            info,
        );
        let mut outcome = IndexMap::new();
        outcome.insert(
            String::from("merged_optional_params"),
            JsonValue::Object(self.merged.optional_params),
        );
        outcome.insert(
            String::from("deployment_model_info"),
            deployment.unwrap_or(JsonValue::Null),
        );
        let model = self.merged.model;
        converted.kwargs.shift_remove("prompt_id");
        converted.kwargs.insert(
            String::from(ASYNC_PROMPT_MERGED_PARAMS),
            JsonValue::Object(outcome),
        );
        converted.input = input;
        converted.model.clone_from(&model);
        converted.custom_llm_provider = Some(final_provider.clone());
        PreparedCall {
            call: converted,
            provider: final_provider,
            model,
        }
    }
}

fn swapped_provider(
    resolver: &dyn ModelResolver,
    call: &ResponsesCreateCall,
    requested: &str,
    swapped_model: &str,
    kwargs: &ParameterMap,
) -> Result<String, AsyncCreateError> {
    let request = SwapRequest {
        original_model: &call.model,
        swapped_model,
        custom_llm_provider: Some(requested),
        prompt_id: text_value(kwargs, "prompt_id"),
    };
    match resolve_swapped_provider(&request, kwargs, resolver) {
        Ok(provider) => Ok(provider),
        Err(PromptError::Swap(PromptSwapError::CredentialBoundary(error))) => {
            Err(AsyncCreateError::boundary(error))
        }
        Err(error) => Err(AsyncCreateError {
            cause: Box::new(AsyncCreateCause::Create(CreateError::Prompt(error))),
            provider: requested.to_owned(),
            model: swapped_model.to_owned(),
        }),
    }
}
