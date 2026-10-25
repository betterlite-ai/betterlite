use std::error::Error;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::error::{ProviderError, ProviderErrorParts};
use crate::openai_base_config::{Headers, Object};
use crate::search::records::SearchResponse;

pub const PERPLEXITY_PARAMS: [&str; 4] = [
    "max_results",
    "search_domain_filter",
    "country",
    "max_tokens_per_page",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchQuery {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Copy)]
pub struct OutboundSearch<'call, Body> {
    pub api_base: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub optional_params: &'call Object,
    pub body: &'call Body,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderResponse<'call> {
    pub status_code: u16,
    pub headers: &'call Headers,
    pub text: &'call str,
}

pub trait SearchProvider {
    type Body: Serialize;
    type Error: Error;

    #[must_use]
    fn ui_friendly_name() -> &'static str {
        "Unknown Search Provider"
    }

    fn supports_rich_search_input(&self) -> bool {
        false
    }

    fn http_method(&self) -> HttpMethod {
        HttpMethod::Post
    }

    fn validate_environment(
        &self,
        headers: Headers,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Headers {
        headers
    }

    fn sign_request(
        &self,
        headers: Headers,
        _request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        Ok((headers, None))
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        optional_params: &Object,
        body: Option<&Self::Body>,
    ) -> Result<String, Self::Error>;

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<Self::Body, Self::Error>;

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Self::Error>;

    fn http_error(&self, response: &ProviderResponse<'_>) -> ProviderError {
        self.error_class(response.text, response.status_code, response.headers)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: &Headers) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: message.to_owned(),
            headers: Some(headers.clone().into_iter().collect()),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}
