use betterlite_config::types::LiteLlmParams;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::count_tokens::error::CountTokensError;
use crate::count_tokens::handler::{CountTokensHandler, HandlerRequest};
use crate::count_tokens::messages::{ChatMessage, messages_to_responses_input};
use crate::count_tokens::python_text::{PyValue, display_text};
use crate::count_tokens::request::ResponsesInput;

const OPENAI_PROVIDER: &str = "openai";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenCountResponse {
    pub total_tokens: i64,
    pub request_model: String,
    pub model_used: String,
    pub tokenizer_type: String,
    pub original_response: Option<Map<String, Value>>,
    pub error: bool,
    pub error_message: Option<String>,
    pub status_code: Option<u16>,
}

#[derive(Debug)]
pub struct CountTokensCall<'call> {
    pub model_to_use: &'call str,
    pub messages: &'call [ChatMessage],
    pub deployment: Option<&'call LiteLlmParams>,
    pub request_model: &'call str,
    pub tools: Option<&'call [Map<String, Value>]>,
    pub system: Option<&'call PyValue>,
}

#[derive(Debug)]
pub struct OpenAiTokenCounter {
    pub handler: CountTokensHandler,
    pub environment_api_key: Option<SecretString>,
}

pub trait TokenCounter {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool;
}

impl TokenCounter for OpenAiTokenCounter {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some(OPENAI_PROVIDER)
    }
}

fn response(call: &CountTokensCall<'_>, total_tokens: i64) -> TokenCountResponse {
    TokenCountResponse {
        total_tokens,
        request_model: call.request_model.to_owned(),
        model_used: call.model_to_use.to_owned(),
        tokenizer_type: String::from("openai_api"),
        original_response: None,
        error: false,
        error_message: None,
        status_code: None,
    }
}

fn failure(call: &CountTokensCall<'_>, error: &CountTokensError) -> TokenCountResponse {
    let (status_code, message) = error.status_and_message();
    tracing::warn!(
        status = status_code,
        message,
        "OpenAI CountTokens API error"
    );
    TokenCountResponse {
        error: true,
        error_message: Some(message),
        status_code: Some(status_code),
        ..response(call, 0)
    }
}

impl OpenAiTokenCounter {
    pub async fn count_tokens(&self, call: &CountTokensCall<'_>) -> Option<TokenCountResponse> {
        if call.messages.is_empty() {
            return None;
        }
        let configured = call.deployment.and_then(|found| found.api_key.as_ref());
        let Some(api_key) = [configured, self.environment_api_key.as_ref()]
            .into_iter()
            .flatten()
            .find(|key| !key.expose_secret().is_empty())
        else {
            tracing::warn!("No OpenAI API key found for token counting");
            return None;
        };
        let (items, extracted) = messages_to_responses_input(call.messages);
        let instructions = extracted.or_else(|| {
            call.system
                .filter(|system| **system != PyValue::Null)
                .map(display_text)
        });
        if items.is_empty() {
            return None;
        }
        let input = ResponsesInput::Items(items);
        let request = HandlerRequest {
            model: call.model_to_use,
            input: &input,
            api_key,
            api_base: call.deployment.and_then(|found| found.api_base.as_deref()),
            timeout: None,
            tools: call.tools,
            instructions: instructions.as_deref(),
        };
        match self.handler.handle_count_tokens_request(&request).await {
            Ok(answer) => answer.map(|counted| TokenCountResponse {
                original_response: Some(counted.original_response),
                ..response(call, counted.input_tokens)
            }),
            Err(error) => Some(failure(call, &error)),
        }
    }
}
