use std::fmt::Display;
use std::time::SystemTime;

use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::error::CountCallError;
use super::handler::{
    CountTokensHandler, CountTokensHandlerError, CountTokensHandlerRequest, ParamMap,
};
use super::reply::CountResponse;

const BEDROCK_PROVIDER: &str = "bedrock";
const TOKENIZER_TYPE: &str = "bedrock_api";
const API_WARNING: &str = "Bedrock CountTokens API error";
const GENERIC_WARNING: &str = "Error calling Bedrock CountTokens API";

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

// LiteLLM hands this counter a deployment dict and reads only its litellm_params, so other keys stay unread
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CountTokensDeployment {
    #[serde(default)]
    pub litellm_params: ParamMap,
}

#[derive(Debug)]
pub struct CountTokensCall<'call> {
    pub model_to_use: &'call str,
    pub messages: Option<&'call Json>,
    pub deployment: Option<&'call CountTokensDeployment>,
    pub request_model: &'call str,
    pub tools: Option<&'call Json>,
    pub system: Option<&'call Json>,
    pub now: SystemTime,
}

pub struct BedrockTokenCounter<H> {
    pub handler: H,
}

pub trait TokenCounter {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool;
}

impl<H> TokenCounter for BedrockTokenCounter<H> {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some(BEDROCK_PROVIDER)
    }
}

impl<H: CountTokensHandler> BedrockTokenCounter<H> {
    pub async fn count_tokens(&self, call: &CountTokensCall<'_>) -> Option<TokenCountResponse> {
        let messages = call.messages?;
        if !has_content(messages) {
            return None;
        }
        let empty = ParamMap::new();
        let parameters = call
            .deployment
            .map_or(&empty, |deployment| &deployment.litellm_params);
        let mut request_data = IndexMap::new();
        request_data.insert("model".to_owned(), Json::Text(call.model_to_use.to_owned()));
        request_data.insert("messages".to_owned(), messages.clone());
        if let Some(tools) = call.tools.filter(|tools| has_content(tools)) {
            request_data.insert("tools".to_owned(), tools.clone());
        }
        if let Some(system) = call.system.filter(|system| has_content(system)) {
            request_data.insert("system".to_owned(), system.clone());
        }
        // LiteLLM resolves the model before its try block, so a broken deployment name never becomes a record
        let resolved_model = get_bedrock_base_model(call.model_to_use);
        let request = CountTokensHandlerRequest {
            request_data: &request_data,
            litellm_params: parameters,
            resolved_model: &resolved_model,
        };
        match self
            .handler
            .handle_count_tokens_request(&request, call.now)
            .await
        {
            Ok(Some(result)) => Some(result_record(call, result)),
            Ok(None) => None,
            Err(error) => Some(handler_failure(call, &error)),
        }
    }
}

fn has_content(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(found) => *found,
        Json::Number(number) => number.as_f64() != Some(0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

fn result_record(call: &CountTokensCall<'_>, result: CountResponse) -> TokenCountResponse {
    match input_tokens(result.get("input_tokens")) {
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

fn input_tokens(value: Option<&Json>) -> Result<i64, CountCallError> {
    match value {
        None => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(CountCallError::InputTokens),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(CountCallError::InputTokens)
        }
    }
}

fn handler_failure(
    call: &CountTokensCall<'_>,
    error: &CountTokensHandlerError,
) -> TokenCountResponse {
    match error {
        CountTokensHandlerError::Record {
            status,
            error: record,
        } => {
            let message = record.error.message.clone();
            tracing::warn!(
                subject = API_WARNING,
                status = *status,
                detail = %message,
                "Bedrock CountTokens API error: status={status}, message={message}"
            );
            error_record(call, *status, message)
        }
        CountTokensHandlerError::Header(source) => generic_failure(call, source),
    }
}

fn generic_failure<Source>(call: &CountTokensCall<'_>, error: &Source) -> TokenCountResponse
where
    Source: Display,
{
    let message = error.to_string();
    tracing::warn!(
        subject = GENERIC_WARNING,
        detail = %message,
        "Error calling Bedrock CountTokens API: {message}"
    );
    error_record(call, 500, message)
}

fn error_record(
    call: &CountTokensCall<'_>,
    status_code: u16,
    message: String,
) -> TokenCountResponse {
    TokenCountResponse {
        total_tokens: 0,
        request_model: call.request_model.to_owned(),
        model_used: call.model_to_use.to_owned(),
        tokenizer_type: TOKENIZER_TYPE.to_owned(),
        original_response: None,
        error: true,
        error_message: Some(message),
        status_code: Some(status_code),
    }
}
