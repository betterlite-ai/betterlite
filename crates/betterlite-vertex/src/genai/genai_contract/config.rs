use std::error::Error;

use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::records::{
    ContentListUnion, GenerateContentConfigDict, GenerateContentRequestDict,
    GenerateContentResponse, ToolConfigDict,
};

pub type RequestHeaders = IndexMap<String, String>;
pub type ResponseHeaders = IndexMap<String, String>;

// get_generate_content_request_top_level_fields: native REST fields that sit beside generationConfig
pub const TOP_LEVEL_FIELDS: [&str; 4] = ["safetySettings", "toolConfig", "cachedContent", "labels"];

#[derive(Debug, Clone, Copy)]
pub struct EnvironmentRequest<'request> {
    pub api_key: Option<&'request SecretString>,
    pub headers: &'request RequestHeaders,
    pub model: &'request str,
    pub litellm_params: Option<&'request Object>,
}

#[derive(Debug, Clone, Copy)]
pub struct GenerateContentRequest<'request> {
    pub model: &'request str,
    pub contents: &'request ContentListUnion,
    pub tools: Option<&'request ToolConfigDict>,
    pub generation_config: &'request GenerateContentConfigDict,
    pub system_instruction: Option<&'request Json>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateContentUpstream {
    pub status: u16,
    pub headers: ResponseHeaders,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct ResponseTransform<'request> {
    pub model: &'request str,
    pub raw_response: &'request GenerateContentUpstream,
}

pub trait BaseGenerateContentConfig: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn supported_optional_params(&self, model: &str) -> Vec<String>;

    fn map_optional_params(
        &self,
        config: &GenerateContentConfigDict,
        model: &str,
    ) -> GenerateContentConfigDict;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, Self::Error>;

    fn transform_request(
        &self,
        request: &GenerateContentRequest<'_>,
    ) -> Result<GenerateContentRequestDict, Self::Error>;

    fn transform_response(
        &self,
        request: &ResponseTransform<'_>,
    ) -> Result<GenerateContentResponse, Self::Error>;

    fn request_top_level_fields(&self) -> [&'static str; 4] {
        TOP_LEVEL_FIELDS
    }

    // LiteLLM's base get_error_class answers by raising BaseLLMException with the status, the message and the headers
    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: error_message.to_owned(),
            headers: Some(
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }

    // LiteLLM's get_auth_token_and_url and its sync twin: a provider answers the headers and the URL it calls
    fn auth_token_and_url(
        &self,
        request: &AuthTokenRequest<'_>,
    ) -> Result<(RequestHeaders, String), Self::Error>;
}

#[derive(Debug, Clone, Copy)]
pub struct AuthTokenRequest<'request> {
    pub api_base: Option<&'request str>,
    pub model: &'request str,
    pub litellm_params: &'request Object,
    pub stream: bool,
}
