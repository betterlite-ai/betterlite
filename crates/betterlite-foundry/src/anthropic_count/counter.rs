use std::future::Future;

use betterlite_anthropic::count_tokens::counter::{TokenCountResponse, TokenCounter};
use betterlite_anthropic::count_tokens::error::CountTokensError;
use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_anthropic::count_tokens::wire::{CountResponse, CountTokensBody};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::anthropic_count::error::ApiError;
use crate::anthropic_count::handler::{CountTokensRequest, FoundryCountHandler};
use crate::anthropic_count::reply::FoundryCountReply;
use crate::anthropic_messages::environment::{ParamsKey, TokenSource};

const AZURE_AI_PROVIDER: &str = "azure_ai";
const TOKENIZER_TYPE: &str = "azure_ai_anthropic_api";

pub struct CountTokensCall<'call> {
    pub model_to_use: &'call str,
    pub messages: Option<&'call Json>,
    pub contents: Option<&'call Json>,
    pub params_key: ParamsKey<'call>,
    pub api_base: Option<&'call str>,
    pub token: &'call TokenSource<'call>,
    pub request_model: &'call str,
    pub tools: Option<&'call Json>,
    pub system: Option<&'call Json>,
}

struct ModelNames {
    request_model: String,
    model_used: String,
}

pub struct AzureAnthropicTokenCounter<T> {
    pub handler: FoundryCountHandler<T>,
    pub environment_api_key: Option<SecretString>,
    pub environment_api_base: Option<String>,
}

impl<T> TokenCounter for AzureAnthropicTokenCounter<T> {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some(AZURE_AI_PROVIDER)
    }
}

impl<T: CountTokensTransport> AzureAnthropicTokenCounter<T> {
    // The call borrows credential sources that are not Sync, so the future only owns the two model names and the handler's future
    pub fn count_tokens<'counter>(
        &'counter self,
        call: &CountTokensCall<'_>,
    ) -> impl Future<Output = Option<TokenCountResponse>> + Send + use<'counter, T> {
        let names = ModelNames {
            request_model: call.request_model.to_owned(),
            model_used: call.model_to_use.to_owned(),
        };
        let started = self.start_count(call);
        async move {
            match started?.await {
                Ok(reply) => success(&names, &reply),
                Err(error) => Some(failure(&names, &error)),
            }
        }
    }

    // None is LiteLLM's early return: no messages, or no key or base to count with
    fn start_count<'counter>(
        &'counter self,
        call: &CountTokensCall<'_>,
    ) -> Option<impl Future<Output = Result<FoundryCountReply, ApiError>> + Send + use<'counter, T>>
    {
        let messages = call.messages?;
        if falsey(messages) {
            return None;
        }
        let (api_key, api_base) = self.credentials(call)?;
        let request = CountTokensRequest {
            body: CountTokensBody {
                model: call.model_to_use,
                messages,
                tools: call.tools,
                system: call.system,
                optional_params: &IndexMap::new(),
            },
            api_key,
            api_base,
            params_key: call.params_key,
            token: call.token,
            timeout: None,
        };
        Some(self.handler.handle_count_tokens_request(&request))
    }

    // A deployment key and base win over the environment pair, and neither has a global fallback
    fn credentials<'call>(
        &'call self,
        call: &'call CountTokensCall<'_>,
    ) -> Option<(&'call SecretString, &'call str)> {
        let deployment_key = match call.params_key {
            ParamsKey::DictAbsent | ParamsKey::DictNull | ParamsKey::Typed(None) => None,
            ParamsKey::Dict(key) | ParamsKey::Typed(Some(key)) => Some(key),
        }
        .filter(|key| !key.expose_secret().is_empty());
        let environment_key = self
            .environment_api_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        let Some(api_key) = deployment_key.or(environment_key) else {
            tracing::warn!("No Azure AI API key found for token counting");
            return None;
        };
        let deployment_base = call.api_base.filter(|base| !base.is_empty());
        let environment_base = self
            .environment_api_base
            .as_deref()
            .filter(|base| !base.is_empty());
        let Some(api_base) = deployment_base.or(environment_base) else {
            tracing::warn!("No Azure AI API base found for token counting");
            return None;
        };
        Some((api_key, api_base))
    }
}

// LiteLLM's `not messages` gate runs before the key check, and an empty dict is falsey too
fn falsey(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(found) => !*found,
        Json::Number(number) => number.as_f64() == Some(0.0_f64),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(fields) => fields.is_empty(),
    }
}

// LiteLLM's counter has no local tokenizer fallback: a null result counts nothing and returns None
fn success(names: &ModelNames, reply: &FoundryCountReply) -> Option<TokenCountResponse> {
    let fields = match &reply.document {
        Json::Object(fields) => fields,
        Json::Null => return None,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Some(generic_failure(names, &CountTokensError::NotAnObject));
        }
    };
    let total_tokens = match input_tokens(fields) {
        Ok(total_tokens) => total_tokens,
        Err(error) => return Some(generic_failure(names, &error)),
    };
    Some(TokenCountResponse {
        total_tokens,
        request_model: names.request_model.clone(),
        model_used: names.model_used.clone(),
        tokenizer_type: TOKENIZER_TYPE.to_owned(),
        original_response: Some(fields.clone()),
        error: false,
        error_message: None,
        status_code: None,
    })
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
fn failure(names: &ModelNames, error: &ApiError) -> TokenCountResponse {
    let status = error.status_code();
    let message = error.to_string();
    tracing::warn!("Azure AI Anthropic CountTokens API error: status={status}, message={message}");
    error_record(names, status, &message)
}

// LiteLLM's except Exception branch, reached when the response has no integer input_tokens
fn generic_failure(names: &ModelNames, error: &CountTokensError) -> TokenCountResponse {
    let message = error.to_string();
    tracing::warn!("Error calling Azure AI Anthropic CountTokens API: {message}");
    error_record(names, 500, &message)
}

fn error_record(names: &ModelNames, status_code: u16, message: &str) -> TokenCountResponse {
    TokenCountResponse {
        total_tokens: 0,
        request_model: names.request_model.clone(),
        model_used: names.model_used.clone(),
        tokenizer_type: TOKENIZER_TYPE.to_owned(),
        original_response: None,
        error: true,
        error_message: Some(message.to_owned()),
        status_code: Some(status_code),
    }
}
