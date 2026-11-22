pub mod error;
pub mod reply;
pub mod request;
mod tests;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::ServerCredential;
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, RequestInput, SearchProvider,
};
use betterlite_openai::search::records::SearchResponse;
use error::TinyfishSearchError;
use request::{CallerLimit, TinyfishBody};
use secrecy::{ExposeSecret as _, SecretString};
use std::sync::{Mutex, PoisonError};
use tracing::warn;
pub const TINYFISH_API_BASE: &str = "https://api.search.tinyfish.ai";
// TinyFish's natural per-page SERP ceiling: max_results only cuts the reply and is never sent
pub const TINYFISH_RESULT_CAP: usize = 10;
const KEY_VARIABLE: &str = "TINYFISH_API_KEY";
const BASE_VARIABLE: &str = "TINYFISH_API_BASE";
const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[KEY_VARIABLE],
    base_var: Some(BASE_VARIABLE),
    default_base: Some(TINYFISH_API_BASE),
};
pub struct TinyfishSearch<'env> {
    lookup: &'env dyn EnvironmentLookup,
    // transform_search_request stashes the caller's limit here for transform_search_response, hence one config per call
    caller_max_results: Mutex<Option<usize>>,
}
impl<'env> TinyfishSearch<'env> {
    pub fn load(lookup: &'env dyn EnvironmentLookup) -> Self {
        Self {
            lookup,
            caller_max_results: Mutex::new(None),
        }
    }
    pub fn validated_headers(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, TinyfishSearchError> {
        let key = CREDENTIAL
            .resolve(api_key.cloned(), api_base, self.lookup)?
            .ok_or(TinyfishSearchError::MissingApiKey)?;
        headers.insert(String::from("X-API-Key"), key.expose_secret().to_owned());
        headers.insert(String::from("Accept"), String::from("application/json"));
        Ok(headers)
    }
    fn remember(&self, limit: CallerLimit) {
        let mut stored = self
            .caller_max_results
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match limit {
            CallerLimit::Set(value) => *stored = Some(value),
            CallerLimit::Ignored(raw) => {
                warn!("TinyFish Search: max_results={raw} is not a valid integer; ignoring.");
            }
            CallerLimit::Absent => {}
        }
    }
}
impl SearchProvider for TinyfishSearch<'_> {
    type Body = TinyfishBody;
    type Error = TinyfishSearchError;
    fn ui_friendly_name() -> &'static str {
        "TinyFish"
    }
    fn http_method(&self) -> HttpMethod {
        HttpMethod::Get
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
        request: &OutboundSearch<'_, TinyfishBody>,
    ) -> Result<(Headers, Option<Vec<u8>>), TinyfishSearchError> {
        let validated = self.validated_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((validated, None))
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        body: Option<&TinyfishBody>,
        _api_key: Option<&SecretString>,
    ) -> Result<String, TinyfishSearchError> {
        let base = api_base
            .filter(|candidate| !candidate.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                self.lookup
                    .lookup(BASE_VARIABLE)
                    .filter(|candidate| !candidate.is_empty())
            })
            .unwrap_or_else(|| TINYFISH_API_BASE.to_owned());
        match body {
            Some(record) => Ok(format!("{base}?{}", request::query_string(&record.params)?)),
            None => Ok(base),
        }
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<TinyfishBody, TinyfishSearchError> {
        let (body, limit) = request::build(input.query, input.optional_params);
        self.remember(limit);
        Ok(body)
    }
    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, TinyfishSearchError> {
        let limit = self
            .caller_max_results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwrap_or(TINYFISH_RESULT_CAP);
        reply::decode_tinyfish_search(response.text, response.status_code, response.headers, limit)
    }
    fn http_error(&self, response: &ProviderResponse<'_>) -> ProviderError {
        reply::wrapped_error(response.text, response.status_code, response.headers)
    }
}
