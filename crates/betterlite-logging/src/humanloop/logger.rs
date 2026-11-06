use std::collections::HashMap;
use std::error::Error;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{
    AllMessageValues, PromptIgnoreFlags, PromptParams, StandardCallbackDynamicParams,
};
use secrecy::SecretString;

use crate::humanloop::compile::{CompileError, compile_prompt};
use crate::humanloop::prompt_cache::{Clock, PromptCache};
use crate::humanloop::prompt_manager::{
    FetchError, HumanLoopPromptManager, PROMPTS_URL, get_model_from_prompt,
};
use crate::humanloop::settings::HumanloopSettings;
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
    pub manager: HumanLoopPromptManager<T, C>,
}

impl<T, C> HumanloopLogger<T, C>
where
    T: PromptTransport + Sync,
    C: Clock + Send,
{
    #[must_use]
    pub fn new(settings: HumanloopSettings, transport: T, clock: C) -> Self {
        let cache_ttl = Duration::from_secs(settings.cache_ttl_seconds);
        Self {
            api_key: settings.api_key,
            manager: HumanLoopPromptManager {
                transport,
                cache: Mutex::new(PromptCache {
                    clock,
                    ttl: cache_ttl,
                    entries: HashMap::new(),
                }),
                prompt_url: PROMPTS_URL.to_owned(),
            },
        }
    }

    // LiteLLM accepts the ignore_prompt_manager_* arguments here and never reads them.
    pub async fn get_chat_completion_prompt(
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
        let prompt_variables = request.prompt_variables;
        let compiled = self
            .manager
            .resolve(prompt_id, api_key, move |prompt| {
                let model = get_model_from_prompt(prompt, request.model);
                compile_prompt(&mut prompt.prompt_template, prompt_variables)?;
                let mut non_default_params = request.non_default_params.clone();
                if let Some(optional_params) = &prompt.prompt_optional_params {
                    non_default_params.extend(optional_params.clone());
                }
                Ok(ChatCompletionPrompt {
                    model,
                    messages: prompt.prompt_template.clone(),
                    non_default_params,
                })
            })
            .await?;
        compiled.map_err(HumanloopError::Compile)
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
