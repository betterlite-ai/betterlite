use betterlite_config::types::LiteLlmParams;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::count_tokens::error::{ApiError, CountTokensError};
use crate::count_tokens::handler::{AnthropicCountTokensHandler, CountTokensTransport};
use crate::count_tokens::validate::is_falsey;
use crate::count_tokens::wire::{CountResponse, CountTokensBody, CountTokensRequest};

const ANTHROPIC_PROVIDER: &str = "anthropic";
const TOKENIZER_TYPE: &str = "anthropic_api";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenCountResponse {
    pub total_tokens: i64,
    pub request_model: String,
    pub model_used: String,
    pub tokenizer_type: String,
    pub original_response: Option<CountResponse>,
    pub error: bool,
    pub error_message: Option<String>,
    pub status_code: Option<u16>,
}

#[derive(Debug)]
pub struct CountTokensCall<'call> {
    pub model_to_use: &'call str,
    pub messages: Option<&'call Json>,
    pub contents: Option<&'call Json>,
    pub deployment: Option<&'call LiteLlmParams>,
    pub request_model: &'call str,
    pub tools: Option<&'call Json>,
    pub system: Option<&'call Json>,
}

#[derive(Debug)]
pub struct AnthropicTokenCounter<T> {
    pub handler: AnthropicCountTokensHandler<T>,
    pub environment_api_key: Option<SecretString>,
}

pub trait TokenCounter {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool;
}

impl<T> TokenCounter for AnthropicTokenCounter<T> {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some(ANTHROPIC_PROVIDER)
    }
}

impl<T: CountTokensTransport> AnthropicTokenCounter<T> {
    pub async fn count_tokens(&self, call: &CountTokensCall<'_>) -> Option<TokenCountResponse> {
        let messages = call.messages?;
        if is_falsey(messages) {
            return None;
        }
        let deployment_key = call
            .deployment
            .and_then(|deployment| deployment.api_key.as_ref())
            .filter(|key| !key.expose_secret().is_empty());
        let environment_key = self
            .environment_api_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        let Some(api_key) = deployment_key.or(environment_key) else {
            tracing::warn!("No Anthropic API key found for token counting");
            return None;
        };
        let request = CountTokensRequest {
            body: CountTokensBody {
                model: call.model_to_use,
                messages,
                tools: call.tools,
                system: call.system,
                optional_params: &IndexMap::new(),
            },
            api_key,
            api_base: None,
            timeout: None,
        };
        match self.handler.handle_count_tokens_request(&request).await {
            Ok(result) => Some(success(call, result)),
            Err(error) => Some(failure(call, &error)),
        }
    }
}

fn success(call: &CountTokensCall<'_>, result: CountResponse) -> TokenCountResponse {
    match input_tokens(&result) {
        Ok(total_tokens) => TokenCountResponse {
            total_tokens,
            request_model: call.request_model.to_owned(),
            model_used: call.model_to_use.to_owned(),
            tokenizer_type: TOKENIZER_TYPE.to_owned(),
            original_response: Some(result),
            error: false,
            error_message: None,
            status_code: None,
        },
        Err(error) => generic_failure(call, &error),
    }
}

fn input_tokens(result: &CountResponse) -> Result<i64, CountTokensError> {
    match result.get("input_tokens") {
        None => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(CountTokensError::InputTokens),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(CountTokensError::InputTokens)
        }
    }
}

// LiteLLM's except AnthropicError branch: the upstream status and the raw response text stay visible
fn failure(call: &CountTokensCall<'_>, error: &ApiError) -> TokenCountResponse {
    let status = error.status_code();
    let message = error.to_string();
    tracing::warn!("Anthropic CountTokens API error: status={status}, message={message}");
    error_record(call, status, &message)
}

// LiteLLM's except Exception branch, reached when the response has no integer input_tokens
fn generic_failure(call: &CountTokensCall<'_>, error: &CountTokensError) -> TokenCountResponse {
    let message = error.to_string();
    tracing::warn!("Error calling Anthropic CountTokens API: {message}");
    error_record(call, 500, &message)
}

fn error_record(call: &CountTokensCall<'_>, status_code: u16, message: &str) -> TokenCountResponse {
    TokenCountResponse {
        total_tokens: 0,
        request_model: call.request_model.to_owned(),
        model_used: call.model_to_use.to_owned(),
        tokenizer_type: TOKENIZER_TYPE.to_owned(),
        original_response: None,
        error: true,
        error_message: Some(message.to_owned()),
        status_code: Some(status_code),
    }
}
