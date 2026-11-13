pub mod error;
mod tests;

use std::error::Error;

use betterlite_anthropic::message_replay::{
    is_invalid_thinking_error, strip_thinking_from_request,
};
use betterlite_core::api::messages_native_prepare::fields::native_supported_fields;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;

use error::{BaseLlmException, MessagesContractError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessagesRequest;
pub struct MessagesResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessagesPrepared;

pub type Headers = IndexMap<String, String>;

// LiteLLM's sign_request returns the headers it was handed for every provider that does not sign requests itself
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedRequest {
    pub headers: Headers,
    pub signed_body: Option<Vec<u8>>,
}

// B962's stream factory fills the real handle; the base config only reports that a subclass skipped one
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessagesStream;

pub trait AnthropicMessagesConfig: Send + Sync {
    type Error: Error + Send + Sync + 'static + From<MessagesContractError>;

    // LiteLLM's custom_llm_provider property; a null provider resolves to anthropic wherever one is needed
    fn custom_llm_provider(&self) -> Option<ProviderId> {
        None
    }

    // The Anthropic subclass owns the list; the provider decides whether compaction is part of it
    fn supported_params(&self, _model: &str) -> Vec<&'static str> {
        native_supported_fields(self.custom_llm_provider())
    }

    fn filters_beta_headers(&self) -> bool {
        true
    }

    fn strips_billing_metadata(&self) -> bool {
        false
    }

    fn sign_request(&self, headers: Headers) -> SignedRequest {
        SignedRequest {
            headers,
            signed_body: None,
        }
    }

    fn complete_url(&self, api_base: Option<&str>) -> String {
        api_base.unwrap_or_default().to_owned()
    }

    fn handles_web_search_natively(&self) -> bool {
        true
    }

    fn uses_provider_api_base(&self) -> bool {
        false
    }

    fn stream_iterator(&self) -> Result<MessagesStream, Self::Error> {
        Err(Self::Error::from(MessagesContractError::NotImplemented {
            message: error::NOT_IMPLEMENTED_STREAM,
        }))
    }

    fn error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: Headers,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers,
        }
    }

    fn max_http_retries(&self) -> u8 {
        2
    }

    fn retries_http_error(&self, status_code: u16, response_text: &str) -> bool {
        status_code == 400 && is_invalid_thinking_error(response_text)
    }

    fn request_after_http_error(
        &self,
        status_code: u16,
        response_text: &str,
        request: &mut Object,
    ) {
        if self.retries_http_error(status_code, response_text) {
            strip_thinking_from_request(request);
        }
    }
}
