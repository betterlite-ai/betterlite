use std::error::Error;
use std::sync::Mutex;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{
    AllMessageValues, PromptIgnoreFlags, PromptParams, StandardCallbackDynamicParams,
};
use secrecy::SecretString;

use crate::humanloop::compile::{CompileError, compile_prompt};
use crate::humanloop::prompt_cache::Clock;
use crate::humanloop::prompt_manager::{FetchError, HumanLoopPromptManager, get_model_from_prompt};
use crate::humanloop::transport::PromptTransport;

#[derive(Debug, thiserror::Error)]
pub enum DynamicKeyError {
    #[error("humanloop_api_key must be a string")]
    NotAString,
}

#[derive(Debug, thiserror::Error)]
pub enum HumanloopError<SendError>
where
    SendError: Error + 'static,
{
    #[error(transparent)]
    Fetch(#[from] FetchError<SendError>),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    DynamicKey(#[from] DynamicKeyError),
    #[error("prompt_id is required for Humanloop integration")]
    MissingPromptId,
    #[error("the Humanloop prompt cache is unavailable")]
    Unavailable,
}

pub struct ChatCompletionPromptRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<AllMessageValues>,
    pub non_default_params: PromptParams,
    pub prompt_id: Option<&'request str>,
    pub prompt_variables: Option<&'request PromptParams>,
    pub dynamic_callback_params: &'request StandardCallbackDynamicParams,
    pub ignore_flags: PromptIgnoreFlags,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChatCompletionPrompt {
    pub model: String,
    pub messages: Vec<AllMessageValues>,
    pub non_default_params: PromptParams,
}

pub struct HumanloopLogger<T, C> {
    pub api_key: Option<SecretString>,
    pub manager: Mutex<HumanLoopPromptManager<T, C>>,
}

impl<T, C> HumanloopLogger<T, C>
where
    T: PromptTransport,
    C: Clock,
{
    // LiteLLM accepts the ignore_prompt_manager_* arguments here and never reads them.
    pub fn get_chat_completion_prompt(
        &self,
        request: ChatCompletionPromptRequest<'_>,
    ) -> Result<ChatCompletionPrompt, HumanloopError<T::Error>> {
        let Some(prompt_id) = request.prompt_id else {
            return Err(HumanloopError::MissingPromptId);
        };
        let dynamic_api_key = dynamic_api_key(request.dynamic_callback_params)?;
        let Some(api_key) = dynamic_api_key.as_ref().or(self.api_key.as_ref()) else {
            return Ok(ChatCompletionPrompt {
                model: request.model.to_owned(),
                messages: request.messages,
                non_default_params: request.non_default_params,
            });
        };
        let mut manager = self
            .manager
            .lock()
            .map_err(|_poisoned| HumanloopError::Unavailable)?;
        let prompt = manager.get_prompt_from_id(prompt_id, api_key)?;
        let model = get_model_from_prompt(prompt, request.model);
        compile_prompt(&mut prompt.prompt_template, request.prompt_variables)?;
        let mut non_default_params = request.non_default_params;
        if let Some(optional_params) = &prompt.prompt_optional_params {
            non_default_params.extend(optional_params.clone());
        }
        let messages = prompt.prompt_template.clone();
        drop(manager);
        Ok(ChatCompletionPrompt {
            model,
            messages,
            non_default_params,
        })
    }
}

fn dynamic_api_key(
    dynamic_callback_params: &StandardCallbackDynamicParams,
) -> Result<Option<SecretString>, DynamicKeyError> {
    let Json::Object(params) = dynamic_callback_params else {
        return Ok(None);
    };
    match params.get("humanloop_api_key") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(key)) if key.is_empty() => Ok(None),
        Some(Json::Text(key)) => Ok(Some(SecretString::from(key.as_str()))),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(DynamicKeyError::NotAString)
        }
    }
}
