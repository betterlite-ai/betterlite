pub mod error;
pub mod reply;
mod tests;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::openai_base_config::{Headers, Object};
use crate::search::credentials::ServerCredential;
use crate::search::provider::{
    OutboundSearch, ProviderResponse, RequestInput, SearchProvider, SearchQuery,
};
use crate::search::records::SearchResponse;
use error::PerplexitySearchError;

pub const PERPLEXITY_API_BASE: &str = "https://api.perplexity.ai";
const API_BASE_VARIABLE: &str = "PERPLEXITY_API_BASE";
// search reads PERPLEXITYAI_API_KEY only, unlike the other Perplexity configs
const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &["PERPLEXITYAI_API_KEY"],
    base_var: Some(API_BASE_VARIABLE),
    default_base: Some(PERPLEXITY_API_BASE),
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PerplexitySearchRequest {
    pub query: SearchQuery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_domain_filter: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_page: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

pub struct PerplexitySearch<'environment> {
    pub environment: &'environment dyn EnvironmentLookup,
}

impl PerplexitySearch<'_> {
    pub fn validated_headers(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, PerplexitySearchError> {
        let key = CREDENTIAL
            .resolve(api_key.cloned(), api_base, self.environment)?
            .ok_or(PerplexitySearchError::MissingApiKey)?;
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
        headers.insert(
            String::from("Content-Type"),
            String::from("application/json"),
        );
        Ok(headers)
    }
}

// a null option counts as unset, and a value of another type names the option
fn option<T>(params: &Object, field: &'static str) -> Result<Option<T>, PerplexitySearchError>
where
    T: DeserializeOwned,
{
    params.get(field).map_or(Ok(None), |value| {
        serde_json::from_value(value.clone())
            .map_err(|source| PerplexitySearchError::OptionType { field, source })
    })
}

impl SearchProvider for PerplexitySearch<'_> {
    type Body = PerplexitySearchRequest;
    type Error = PerplexitySearchError;

    fn ui_friendly_name() -> &'static str {
        "Perplexity"
    }

    // B3270's hook cannot fail, so the missing-key error is raised in sign_request before any request leaves
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        self.validated_headers(headers.clone(), api_key, api_base)
            .unwrap_or(headers)
    }

    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        let validated = self.validated_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((validated, None))
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&Self::Body>,
        _api_key: Option<&SecretString>,
    ) -> Result<String, Self::Error> {
        let base = api_base
            .filter(|base| !base.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                self.environment
                    .lookup(API_BASE_VARIABLE)
                    .filter(|base| !base.is_empty())
            })
            .unwrap_or_else(|| PERPLEXITY_API_BASE.to_owned());
        if base.ends_with("/search") {
            return Ok(base);
        }
        Ok(format!("{base}/search"))
    }

    // only the four Perplexity search options are sent; every other optional param is dropped
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<Self::Body, Self::Error> {
        Ok(PerplexitySearchRequest {
            query: input.query.clone(),
            max_results: option(input.optional_params, "max_results")?,
            search_domain_filter: option(input.optional_params, "search_domain_filter")?,
            max_tokens_per_page: option(input.optional_params, "max_tokens_per_page")?,
            country: option(input.optional_params, "country")?,
        })
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Self::Error> {
        reply::decode_search_reply(response.text.as_bytes())
    }
}
