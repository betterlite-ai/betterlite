use super::super::config::{SearchBody, SearchHooks};
use super::super::error::SearchConfigError;
use crate::api::search_providers::linkup::{LinkupSearchConfig, decode_linkup_reply};
use crate::api::search_providers::searxng::{
    SearxngSearchConfig, SearxngSearchRequest, decode_searxng_reply,
};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    HttpMethod, ProviderResponse, RequestInput, SearchProvider,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::SecretString;
use std::sync::{Mutex, PoisonError};
// Linkup resolves caller key and base at load time, so every hook loads the config with its own call's inputs
pub(super) struct LinkupProvider<'env> {
    pub(super) environment: &'env dyn EnvironmentLookup,
}
impl LinkupProvider<'_> {
    fn config(
        &self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<LinkupSearchConfig, SearchConfigError> {
        LinkupSearchConfig::load(api_key.cloned(), api_base, self.environment)
            .map_err(SearchConfigError::provider)
    }
}
impl SearchHooks for LinkupProvider<'_> {
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        let Ok(config) = self.config(api_key, api_base) else {
            return headers;
        };
        config.validate_environment(headers, api_key, api_base)
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        optional_params: &Object,
        _body: Option<&SearchBody>,
        api_key: Option<&SecretString>,
    ) -> Result<String, SearchConfigError> {
        self.config(api_key, api_base)?
            .complete_url(api_base, optional_params, None, api_key)
            .map_err(SearchConfigError::provider)
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError> {
        let typed = self
            .config(input.api_key, input.api_base)?
            .transform_search_request(input)
            .map_err(SearchConfigError::provider)?;
        let encoded =
            serde_json::to_value(&typed).map_err(|source| SearchConfigError::Encode { source })?;
        Ok(SearchBody(encoded))
    }
    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, SearchConfigError> {
        decode_linkup_reply(response.text.as_bytes()).map_err(SearchConfigError::provider)
    }
}
// SearXNG resolves the caller base at load time and builds its query string from the typed body
pub(super) struct SearxngProvider<'env> {
    pub(super) environment: &'env dyn EnvironmentLookup,
    pub(super) body: Mutex<Option<SearxngSearchRequest>>,
}
impl SearxngProvider<'_> {
    fn config(
        &self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<SearxngSearchConfig, SearchConfigError> {
        SearxngSearchConfig::load(api_key.cloned(), api_base, self.environment)
            .map_err(SearchConfigError::provider)
    }
}
impl SearchHooks for SearxngProvider<'_> {
    fn http_method(&self) -> HttpMethod {
        HttpMethod::Get
    }
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        let Ok(config) = self.config(api_key, api_base) else {
            return headers;
        };
        config.validate_environment(headers, api_key, api_base)
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        optional_params: &Object,
        _body: Option<&SearchBody>,
        api_key: Option<&SecretString>,
    ) -> Result<String, SearchConfigError> {
        let config = self.config(api_key, api_base)?;
        let stored = self.body.lock().unwrap_or_else(PoisonError::into_inner);
        config
            .complete_url(api_base, optional_params, stored.as_ref(), api_key)
            .map_err(SearchConfigError::provider)
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError> {
        let typed = self
            .config(input.api_key, input.api_base)?
            .transform_search_request(input)
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
        decode_searxng_reply(response.text.as_bytes()).map_err(SearchConfigError::provider)
    }
}
