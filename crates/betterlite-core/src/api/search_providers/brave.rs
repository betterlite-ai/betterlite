use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::{CredentialError, ServerCredential};
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use chrono::{DateTime, NaiveDate, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

pub mod dates;
pub mod reply;
pub mod request;

use super::google_pse::wire::WireError;
use request::BraveBody;

pub const BRAVE_API_BASE: &str = "https://api.search.brave.com/res/v1/web/search";
const API_KEY_VARIABLE: &str = "BRAVE_API_KEY";
const API_BASE_VARIABLE: &str = "BRAVE_API_BASE";
const SUBSCRIPTION_HEADER: &str = "X-Subscription-Token";
const ACCEPT_HEADER: &str = "Accept";
const ACCEPT_ENCODING_HEADER: &str = "Accept-Encoding";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";

const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[API_KEY_VARIABLE],
    base_var: Some(API_BASE_VARIABLE),
    default_base: Some(BRAVE_API_BASE),
};

#[derive(Debug, Error)]
pub enum BraveSearchError {
    #[error("BRAVE_API_KEY is not set. Set `BRAVE_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the Brave search option `{field}` has the wrong type")]
    OptionType { field: String },
    #[error("the Brave search reply does not match the Brave schema")]
    ReplyDecode(#[source] serde_json::Error),
    #[error("the Brave search reply field `{field}` is not text")]
    ReplyText {
        field: &'static str,
        source: serde_json::Error,
    },
    #[error("the Brave search request count `{value}` is not an integer")]
    CountNotInteger { value: String },
    #[error(transparent)]
    Wire(#[from] WireError),
}

#[derive(Debug)]
pub struct BraveSearchConfig {
    api_key: Option<SecretString>,
    api_base: String,
    // the reply hook sees no request, so the last complete URL stands in for the request's own
    request_url: Mutex<Option<String>>,
    today: NaiveDate,
}

impl BraveSearchConfig {
    // the day is injected by tests and the oracle, and read from the clock otherwise
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup, today: Option<NaiveDate>) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(API_KEY_VARIABLE).map(SecretString::from),
            api_base: present(API_BASE_VARIABLE).unwrap_or_else(|| BRAVE_API_BASE.to_owned()),
            request_url: Mutex::new(None),
            today: today.unwrap_or_else(now_date),
        }
    }

    fn validated_headers(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, BraveSearchError> {
        let Some(resolved) = CREDENTIAL.resolve(api_key.cloned(), api_base, self)? else {
            return Err(BraveSearchError::MissingApiKey);
        };
        let token = resolved.expose_secret().to_owned();
        headers.insert(SUBSCRIPTION_HEADER.to_owned(), token);
        headers.insert(ACCEPT_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        headers.insert(ACCEPT_ENCODING_HEADER.to_owned(), "gzip".to_owned());
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        Ok(headers)
    }

    fn remember_url(&self, url: &str) {
        *self
            .request_url
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(url.to_owned());
    }
}

impl EnvironmentLookup for BraveSearchConfig {
    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            API_KEY_VARIABLE => self
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
            API_BASE_VARIABLE => Some(self.api_base.clone()),
            _ => None,
        }
    }
}

impl SearchProvider for BraveSearchConfig {
    type Body = BraveBody;
    type Error = BraveSearchError;

    fn ui_friendly_name() -> &'static str {
        "Brave Search"
    }

    fn http_method(&self) -> HttpMethod {
        HttpMethod::Get
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
        body: Option<&Self::Body>,
    ) -> Result<String, Self::Error> {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(&self.api_base);
        let Some(payload) = body else {
            self.remember_url(base);
            return Ok(base.to_owned());
        };
        let url = format!("{base}?{}", request::query_string(&payload.params)?);
        self.remember_url(&url);
        Ok(url)
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
        let url = self
            .request_url
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        reply::decode_brave_search(response.text.as_bytes(), url.as_deref(), self.today)
    }
}

fn now_date() -> NaiveDate {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).ok();
    let seconds = elapsed.and_then(|duration| i64::try_from(duration.as_secs()).ok());
    let moment = seconds.and_then(|whole| DateTime::<Utc>::from_timestamp(whole, 0));
    moment.map_or_else(NaiveDate::default, |fixed| fixed.date_naive())
}

mod tests;
