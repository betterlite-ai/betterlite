use super::super::config::{SearchBody, SearchHooks};
use super::super::error::SearchConfigError;
use crate::api::search_providers::google_pse::{
    GooglePseBody, GooglePseSearch, decode_reply as decode_google_pse_reply,
};
use crate::api::search_providers::searchapi::{
    SearchApiBody, SearchApiSearch, decode_reply as decode_searchapi_reply,
};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{HttpMethod, ProviderResponse, RequestInput};
use betterlite_openai::search::records::SearchResponse;
use secrecy::SecretString;
use std::sync::{Mutex, PoisonError};
// Google PSE and SearchAPI keep their key in the URL, so only the three request hooks have work to do
pub(super) struct GooglePseProvider<'env> {
    pub(super) environment: &'env dyn EnvironmentLookup,
    pub(super) body: Mutex<Option<GooglePseBody>>,
}
impl GooglePseProvider<'_> {
    fn config<'call>(
        &'call self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> GooglePseSearch<'call> {
        GooglePseSearch {
            lookup: self.environment,
            api_key: api_key.cloned(),
            api_base: api_base.map(str::to_owned),
            search_engine_id: None,
        }
    }
}
impl SearchHooks for GooglePseProvider<'_> {
    fn http_method(&self) -> HttpMethod {
        HttpMethod::Get
    }
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        self.config(api_key, api_base)
            .validate_environment(headers.clone())
            .unwrap_or(headers)
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&SearchBody>,
        api_key: Option<&SecretString>,
    ) -> Result<String, SearchConfigError> {
        let stored = self.body.lock().unwrap_or_else(PoisonError::into_inner);
        self.config(api_key, api_base)
            .complete_url(api_base, stored.as_ref())
            .map_err(SearchConfigError::provider)
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError> {
        let typed = self
            .config(input.api_key, input.api_base)
            .transform_search_request(input.query, input.optional_params)
            .map_err(SearchConfigError::provider)?;
        let encoded =
            serde_json::to_value(&typed).map_err(|source| SearchConfigError::Encode { source })?;
        *self.body.lock().unwrap_or_else(PoisonError::into_inner) = Some(typed);
        Ok(SearchBody(encoded))
    }
    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, SearchConfigError> {
        decode_google_pse_reply(response.text).map_err(SearchConfigError::provider)
    }
}
pub(super) struct SearchApiProvider<'env> {
    pub(super) environment: &'env dyn EnvironmentLookup,
    pub(super) body: Mutex<Option<SearchApiBody>>,
}
impl SearchApiProvider<'_> {
    fn config<'call>(
        &'call self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> SearchApiSearch<'call> {
        SearchApiSearch {
            lookup: self.environment,
            api_key: api_key.cloned(),
            api_base: api_base.map(str::to_owned),
        }
    }
}
impl SearchHooks for SearchApiProvider<'_> {
    fn http_method(&self) -> HttpMethod {
        HttpMethod::Get
    }
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        self.config(api_key, api_base)
            .validate_environment(headers.clone())
            .unwrap_or(headers)
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&SearchBody>,
        api_key: Option<&SecretString>,
    ) -> Result<String, SearchConfigError> {
        let stored = self.body.lock().unwrap_or_else(PoisonError::into_inner);
        self.config(api_key, api_base)
            .complete_url(api_base, stored.as_ref())
            .map_err(SearchConfigError::provider)
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError> {
        let typed = self
            .config(input.api_key, input.api_base)
            .transform_search_request(input.query, input.optional_params)
            .map_err(SearchConfigError::provider)?;
        let encoded =
            serde_json::to_value(&typed).map_err(|source| SearchConfigError::Encode { source })?;
        *self.body.lock().unwrap_or_else(PoisonError::into_inner) = Some(typed);
        Ok(SearchBody(encoded))
    }
    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, SearchConfigError> {
        decode_searchapi_reply(response.text).map_err(SearchConfigError::provider)
    }
}
