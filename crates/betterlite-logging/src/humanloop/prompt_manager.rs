use std::error::Error;
use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{PromptManagementClient, PromptParams};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::SecretString;

use crate::humanloop::params::OPENAI_CHAT_COMPLETION_PARAMS;
use crate::humanloop::prompt_cache::{Clock, PromptCache};
use crate::humanloop::transport::{PromptFetch, PromptTransport};

pub const INTEGRATION_NAME: &str = "humanloop";
pub(crate) const PROMPTS_URL: &str = "https://api.humanloop.com/v5/prompts/";
const API_KEY_HEADER: &str = "X-Api-Key";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const PROMPT_ID_UNRESERVED: AsciiSet = NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'~');

#[derive(Debug, thiserror::Error)]
pub enum FetchError<SendError>
where
    SendError: Error + 'static,
{
    #[error("Error getting prompt from Humanloop: {text}")]
    Api { text: String },
    #[error("Invalid template message type: {type_name}")]
    InvalidTemplateType { type_name: &'static str },
    #[error("the Humanloop response has no {field} field")]
    MissingField { field: &'static str },
    #[error("the Humanloop response field model must be a string or null")]
    ModelType,
    #[error("the Humanloop response is not a JSON object")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request to Humanloop failed")]
    Transport {
        #[source]
        source: SendError,
    },
}

#[derive(Debug)]
pub struct HumanLoopPromptManager<T, C> {
    pub transport: T,
    pub cache: Mutex<PromptCache<C>>,
    pub prompt_url: String,
}

impl<T, C> HumanLoopPromptManager<T, C>
where
    T: PromptTransport + Sync,
    C: Clock + Send,
{
    pub async fn resolve<R, F>(
        &self,
        humanloop_prompt_id: &str,
        humanloop_api_key: &SecretString,
        mut use_prompt: F,
    ) -> Result<R, FetchError<T::Error>>
    where
        F: FnMut(&mut PromptManagementClient) -> R + Send,
    {
        if let Some(result) = self.with_cached(humanloop_prompt_id, &mut use_prompt) {
            return Ok(result);
        }
        let prompt = fetch_prompt(
            &self.transport,
            &self.prompt_url,
            humanloop_prompt_id,
            humanloop_api_key,
        )
        .await?;
        Ok(self.store_and(humanloop_prompt_id, prompt, &mut use_prompt))
    }

    fn with_cached<R, F>(&self, key: &str, use_prompt: F) -> Option<R>
    where
        F: FnOnce(&mut PromptManagementClient) -> R,
    {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let prompt = cache.peek(key)?;
        let result = use_prompt(prompt);
        drop(cache);
        Some(result)
    }

    fn store_and<R, F>(&self, key: &str, prompt: PromptManagementClient, use_prompt: F) -> R
    where
        F: FnOnce(&mut PromptManagementClient) -> R,
    {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = cache.store(key, prompt);
        let result = use_prompt(entry);
        drop(cache);
        result
    }
}

async fn fetch_prompt<T>(
    transport: &T,
    prompt_url: &str,
    humanloop_prompt_id: &str,
    humanloop_api_key: &SecretString,
) -> Result<PromptManagementClient, FetchError<T::Error>>
where
    T: PromptTransport + Sync,
{
    let fetch = PromptFetch {
        url: format!(
            "{prompt_url}{}",
            utf8_percent_encode(humanloop_prompt_id, &PROMPT_ID_UNRESERVED)
        ),
        headers: vec![
            (API_KEY_HEADER, humanloop_api_key.clone()),
            (CONTENT_TYPE_HEADER, SecretString::from("application/json")),
        ],
    };
    let response = transport
        .get(&fetch)
        .await
        .map_err(|source| FetchError::Transport { source })?;
    if !(200..300).contains(&response.status) {
        return Err(FetchError::Api {
            text: response.body,
        });
    }
    parse_prompt(humanloop_prompt_id, &response.body)
}

fn parse_prompt<SendError>(
    humanloop_prompt_id: &str,
    body: &str,
) -> Result<PromptManagementClient, FetchError<SendError>>
where
    SendError: Error + 'static,
{
    let mut json_response: PromptParams =
        serde_json::from_str(body).map_err(|source| FetchError::Decode { source })?;
    let template_message = json_response
        .shift_remove("template")
        .ok_or(FetchError::MissingField { field: "template" })?;
    let prompt_template = match template_message {
        Json::Object(_) => vec![template_message],
        Json::List(template_messages) => template_messages,
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) => {
            return Err(FetchError::InvalidTemplateType {
                type_name: value_type_name(&other),
            });
        }
    };
    let prompt_template_model = match json_response.shift_remove("model") {
        None => return Err(FetchError::MissingField { field: "model" }),
        Some(Json::Null) => None,
        Some(Json::Text(model)) => Some(model),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(FetchError::ModelType);
        }
    };
    json_response.retain(|name, _value| OPENAI_CHAT_COMPLETION_PARAMS.contains(&name.as_str()));
    Ok(PromptManagementClient {
        prompt_id: Some(humanloop_prompt_id.to_owned()),
        prompt_template,
        prompt_template_model,
        prompt_optional_params: Some(json_response),
        completed_messages: None,
    })
}

// The ValueError names Python's class, so the message keeps that spelling.
fn value_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "<class 'NoneType'>",
        Json::Bool(_) => "<class 'bool'>",
        Json::Number(number) if number.is_f64() => "<class 'float'>",
        Json::Number(_) => "<class 'int'>",
        Json::Text(_) => "<class 'str'>",
        Json::List(_) => "<class 'list'>",
        Json::Object(_) => "<class 'dict'>",
    }
}

#[must_use]
pub fn get_model_from_prompt(
    prompt_management_client: &PromptManagementClient,
    model: &str,
) -> String {
    prompt_management_client
        .prompt_template_model
        .as_ref()
        .map_or_else(
            || model.replace(&format!("{INTEGRATION_NAME}/"), ""),
            String::clone,
        )
}
