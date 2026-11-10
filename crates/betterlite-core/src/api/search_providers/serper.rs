use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::{CredentialError, ServerCredential};
use betterlite_openai::search::provider::{
    OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod reply;

mod request;
mod tests;

pub const SERPER_API_BASE: &str = "https://google.serper.dev";
const API_KEY_VARIABLE: &str = "SERPER_API_KEY";
const API_BASE_VARIABLE: &str = "SERPER_API_BASE";
const API_KEY_HEADER: &str = "X-API-KEY";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";
const SEARCH_SUFFIX: &str = "/search";

const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[API_KEY_VARIABLE],
    base_var: Some(API_BASE_VARIABLE),
    default_base: Some(SERPER_API_BASE),
};

#[derive(Debug, Error)]
pub enum SerperSearchError {
    #[error("SERPER_API_KEY is not set. Set `SERPER_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the Serper search option `{field}` is not a string")]
    OptionType { field: &'static str },
    #[error("the Serper search option `{field}` is not a list of strings")]
    OptionList { field: &'static str },
    #[error("the Serper search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the Serper search reply does not match the Serper schema")]
    ReplyDecode(#[source] serde_json::Error),
    #[error("the Serper search reply field `organic` is null")]
    OrganicShape,
    #[error("the Serper search reply result field `{field}` is null")]
    ResultField { field: &'static str },
    #[error("the Serper search reply result field `{field}` has the wrong type")]
    ReplyField {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Serper search reply holds a value that is not a JSON object")]
    NotAnObject,
}

#[derive(Debug, Clone)]
pub struct SerperSearchSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl SerperSearchSettings {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(API_KEY_VARIABLE).map(SecretString::from),
            api_base: present(API_BASE_VARIABLE).unwrap_or_else(|| SERPER_API_BASE.to_owned()),
        }
    }
}

impl EnvironmentLookup for SerperSearchSettings {
    fn lookup(&self, name: &str) -> Option<String> {
        if name == API_KEY_VARIABLE {
            return self
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned());
        }
        (name == API_BASE_VARIABLE).then(|| self.api_base.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerperSearchRequest {
    #[serde(rename = "q")]
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gl: Option<String>,
    // num and the passthrough options, copied as given
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Debug, Clone)]
pub struct SerperSearchConfig {
    pub settings: SerperSearchSettings,
}

impl SerperSearchConfig {
    pub fn validated_headers(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, SerperSearchError> {
        let Some(key) = CREDENTIAL.resolve(api_key.cloned(), api_base, &self.settings)? else {
            return Err(SerperSearchError::MissingApiKey);
        };
        headers.insert(API_KEY_HEADER.to_owned(), key.expose_secret().to_owned());
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        Ok(headers)
    }
}

impl SearchProvider for SerperSearchConfig {
    type Body = SerperSearchRequest;
    type Error = SerperSearchError;

    fn ui_friendly_name() -> &'static str {
        "Serper"
    }

    // B3270's hook returns headers only; the same check fails in sign_request before any request leaves
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        self.validated_headers(headers.clone(), api_key, api_base)
            .unwrap_or(headers)
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&Self::Body>,
    ) -> Result<String, Self::Error> {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(&self.settings.api_base)
            .trim_end_matches('/');
        if base.ends_with(SEARCH_SUFFIX) {
            return Ok(base.to_owned());
        }
        Ok(format!("{base}{SEARCH_SUFFIX}"))
    }

    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        let validated = self.validated_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((validated, None))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<Self::Body, Self::Error> {
        request::build(query, optional_params)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Self::Error> {
        reply::decode_serper_search(response.text.as_bytes())
    }
}
