use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::{CredentialError, ServerCredential};
use betterlite_openai::search::provider::{
    OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;

pub mod reply;

mod request;
mod tests;

pub const EXA_AI_API_BASE: &str = "https://api.exa.ai";
const API_KEY_VARIABLE: &str = "EXA_API_KEY";
const API_BASE_VARIABLE: &str = "EXA_API_BASE";
const API_KEY_HEADER: &str = "x-api-key";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";
const SEARCH_SUFFIX: &str = "/search";

const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[API_KEY_VARIABLE],
    base_var: Some(API_BASE_VARIABLE),
    default_base: Some(EXA_AI_API_BASE),
};

#[derive(Debug, Error)]
pub enum ExaAiSearchError {
    #[error("EXA_API_KEY is not set. Set `EXA_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the Exa AI search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the Exa AI search reply does not match the Exa AI schema")]
    ReplyDecode(#[source] serde_json::Error),
    #[error("the Exa AI search reply field `results` is null")]
    ResultsShape,
    #[error("the Exa AI search reply result field `{field}` is null")]
    ResultField { field: &'static str },
    #[error("the Exa AI search reply result field `{field}` has the wrong type")]
    ReplyField {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Exa AI search reply holds a value that is not a JSON object")]
    NotAnObject,
}

#[derive(Debug, Clone)]
pub struct ExaAiSearchSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl ExaAiSearchSettings {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(API_KEY_VARIABLE).map(SecretString::from),
            api_base: present(API_BASE_VARIABLE).unwrap_or_else(|| EXA_AI_API_BASE.to_owned()),
        }
    }
}

impl EnvironmentLookup for ExaAiSearchSettings {
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

#[derive(Debug, Clone, Serialize)]
pub struct DefaultContents {
    pub text: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExaAiSearchRequest {
    pub query: String,
    // set only when the caller sent no contents, so it never meets a caller contents in fields
    #[serde(rename = "contents", skip_serializing_if = "Option::is_none")]
    pub default_contents: Option<DefaultContents>,
    // numResults, includeDomains, userLocation, a caller contents and the passthrough options, as given
    #[serde(flatten)]
    pub fields: Object,
}

#[derive(Debug, Clone)]
pub struct ExaAiSearchConfig {
    pub settings: ExaAiSearchSettings,
}

impl ExaAiSearchConfig {
    pub fn validated_headers(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, ExaAiSearchError> {
        let Some(key) = CREDENTIAL.resolve(api_key.cloned(), api_base, &self.settings)? else {
            return Err(ExaAiSearchError::MissingApiKey);
        };
        headers.insert(API_KEY_HEADER.to_owned(), key.expose_secret().to_owned());
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        Ok(headers)
    }
}

impl SearchProvider for ExaAiSearchConfig {
    type Body = ExaAiSearchRequest;
    type Error = ExaAiSearchError;

    fn ui_friendly_name() -> &'static str {
        "Exa AI"
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
            .unwrap_or(&self.settings.api_base);
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
        Ok(request::build(query, optional_params))
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Self::Error> {
        reply::decode_exa_ai_search(response.text.as_bytes())
    }
}
