use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::response_input::ResponsesInput;

use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::error::CreateError;
use crate::api::responses_parameters::error::ResponsesParametersError;
use crate::api::responses_parameters::prompt_merge::{
    ClientMessage, input_to_chat_messages, merge_prompt_management_input,
};
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prefixes::pop_use_chat_completions_api;
use crate::api::responses_provider_selection::prompting::swap::{
    PromptSwapError, SwapRequest, model_info_after_swap, resolve_swapped_provider,
};
use crate::api::responses_provider_selection::prompting::{
    CARRY_UNMATCHED_MESSAGE_POINTS, HookRequest, MergeAnswer, MergeRequest, PromptError,
    PromptManagerError, ProvisionalBridge, api_base_kwarg, will_use_chat_bridge,
};
use crate::api::responses_provider_selection::provider_config::{
    ASYNC_PROMPT_MERGED_PARAMS, ConfigLookup,
};
use crate::api::responses_provider_selection::resolution::ModelResolver;

use super::deps::{AsyncCreateDeps, AsyncPromptPort};
use super::error::AsyncCreateError;
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
        return Ok(PreparedCall::settled(converted, provider, &call.model));
    }
    let client_input = match input_json(&call.input) {
        Ok(input) => match input_to_chat_messages(Some(&input)) {
            Ok(messages) => messages,
            Err(error) => return Err(merge_failure(error, &provider, &call.model)),
        },
        Err(error) => return Err(AsyncCreateError::create(error, &provider, &call.model)),
    };
    let bridged = bridges(deps.lookup, call, &provider, &kwargs);
    let answer = match run_hook(port, call, &mut kwargs, &client_input, bridged).await {
        Ok(answer) => answer,
        Err(error) => {
            let cause = CreateError::Prompt(PromptError::Manager(error));
            return Err(AsyncCreateError::create(cause, &provider, &call.model));
        }
    };
    let state = Answer {
        call,
        provider,
        kwargs,
        answer,
    };
    let input = state.input()?;
    let final_provider = state.provider(deps.resolver)?;
    Ok(state.outcome(input, final_provider, converted))
}

fn merge_failure(error: ResponsesParametersError, provider: &str, model: &str) -> AsyncCreateError {
    let cause = CreateError::Prompt(PromptError::Merge(error));
    AsyncCreateError::create(cause, provider, model)
}

// LiteLLM's kwargs at the hook: the caller's extras plus the base and key the call carries as named arguments
fn connection_kwargs(call: &ResponsesCreateCall) -> ParameterMap {
    let mut kwargs = call.kwargs.clone();
    if let Some(base) = &call.api_base {
        kwargs.insert(String::from("api_base"), JsonValue::Text(base.clone()));
    }
    if let Some(key) = &call.api_key {
        let secret = JsonValue::Text(key.expose_secret().to_owned());
        kwargs.insert(String::from("api_key"), secret);
    }
    if let Some(verify) = call.ssl_verify {
        kwargs.insert(String::from("ssl_verify"), JsonValue::Bool(verify));
    }
    kwargs
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

fn text_value<'map>(kwargs: &'map ParameterMap, name: &str) -> Option<&'map str> {
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

fn input_json(input: &ResponsesInput) -> Result<JsonValue, CreateError> {
    transcode(input).map_err(CreateError::from)
}

fn merge_request<'request>(
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
    let answer = port.async_get_chat_completion_prompt(&request).await;
    if bridged {
        kwargs.shift_remove(CARRY_UNMATCHED_MESSAGE_POINTS);
    }
    answer
}

struct Answer<'call> {
    call: &'call ResponsesCreateCall,
    provider: String,
    kwargs: ParameterMap,
    answer: MergeAnswer,
}

impl Answer<'_> {
    fn input(&self) -> Result<ResponsesInput, AsyncCreateError> {
        let original = input_json(&self.call.input)
            .map_err(|error| AsyncCreateError::create(error, &self.provider, &self.answer.model))?;
        let merged = match merge_prompt_management_input(&original, &self.answer.messages) {
            Ok(outcome) => outcome.input,
            Err(error) => return Err(merge_failure(error, &self.provider, &self.answer.model)),
        };
        transcode(&merged).map_err(|error| {
            AsyncCreateError::create(
                CreateError::Rewrite(error),
                &self.provider,
                &self.answer.model,
            )
        })
    }

    fn provider(&self, resolver: &dyn ModelResolver) -> Result<String, AsyncCreateError> {
        if self.answer.model == self.call.model {
            return Ok(self.provider.clone());
        }
        swapped_provider(
            resolver,
            self.call,
            &self.provider,
            &self.answer.model,
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
            JsonValue::Object(self.answer.optional_params),
        );
        outcome.insert(
            String::from("deployment_model_info"),
            deployment.unwrap_or(JsonValue::Null),
        );
        let model = self.answer.model;
        converted.kwargs.shift_remove("prompt_id");
        converted.kwargs.insert(
            String::from(ASYNC_PROMPT_MERGED_PARAMS),
            JsonValue::Object(outcome),
        );
        converted.input = input;
        converted.model = model.clone();
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
        Err(error) => Err(AsyncCreateError::create(
            CreateError::Prompt(error),
            requested,
            swapped_model,
        )),
    }
}
