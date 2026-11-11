use betterlite_models::prompt_contracts::{AllMessageValues, PromptParams};

use crate::orchestration::prompt_orchestration::BoxFuture;

pub const LITELLM_AGENT_PREFIX: &str = "litellm_agent/";

#[derive(Clone, Copy, Debug, Default)]
pub struct LitellmAgentResolver;

impl LitellmAgentResolver {
    #[must_use]
    pub fn get_chat_completion_prompt(
        &self,
        model: &str,
        messages: Vec<AllMessageValues>,
        non_default_params: PromptParams,
        ignore_prompt_manager_model: bool,
    ) -> (String, Vec<AllMessageValues>, PromptParams) {
        if ignore_prompt_manager_model {
            return (model.to_owned(), messages, non_default_params);
        }
        (
            model.replacen(LITELLM_AGENT_PREFIX, "", 1),
            messages,
            non_default_params,
        )
    }

    // LiteLLM's async_get_chat_completion_prompt; the 30-character name rule shortens it.
    #[must_use]
    pub fn async_completion_prompt<'request>(
        &'request self,
        model: &'request str,
        messages: Vec<AllMessageValues>,
        non_default_params: PromptParams,
        ignore_prompt_manager_model: bool,
    ) -> BoxFuture<'request, (String, Vec<AllMessageValues>, PromptParams)> {
        Box::pin(async move {
            self.get_chat_completion_prompt(
                model,
                messages,
                non_default_params,
                ignore_prompt_manager_model,
            )
        })
    }
}

// A model entry selects the resolver by its prefix, unlike the strip which finds the text anywhere.
#[must_use]
pub fn is_litellm_agent_model(model: &str) -> bool {
    if model.starts_with(LITELLM_AGENT_PREFIX) {
        return true;
    }
    false
}

mod tests;
