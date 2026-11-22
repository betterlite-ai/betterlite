use super::super::config::{SearchBody, SearchHooks};
use super::super::error::SearchConfigError;
use super::shared::encoded_body;
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
// Linkup and SearXNG resolve the caller key and base at load time, so every hook loads its config again
pub(super) struct ReloadedProvider<'env> {
    pub(super) environment: &'env dyn EnvironmentLookup,
    pub(super) flavor: ReloadedFlavor,
    pub(super) body: Mutex<Option<SearxngSearchRequest>>,
}
#[derive(Clone, Copy)]
pub(super) enum ReloadedFlavor {
    Linkup,
    Searxng,
}
enum LoadedSearch {
    Linkup(LinkupSearchConfig),
    Searxng(SearxngSearchConfig),
}
impl ReloadedProvider<'_> {
    fn reload(
        &self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<LoadedSearch, SearchConfigError> {
        match self.flavor {
            ReloadedFlavor::Linkup => {
                LinkupSearchConfig::load(api_key.cloned(), api_base, self.environment)
                    .map(LoadedSearch::Linkup)
                    .map_err(|source| SearchConfigError::Provider(Box::new(source)))
            }
            ReloadedFlavor::Searxng => {
                SearxngSearchConfig::load(api_key.cloned(), api_base, self.environment)
                    .map(LoadedSearch::Searxng)
                    .map_err(|source| SearchConfigError::Provider(Box::new(source)))
            }
        }
    }
}
impl SearchHooks for ReloadedProvider<'_> {
    fn http_method(&self) -> HttpMethod {
        match self.flavor {
            ReloadedFlavor::Linkup => HttpMethod::Post,
            ReloadedFlavor::Searxng => HttpMethod::Get,
        }
    }
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        let Ok(loaded) = self.reload(api_key, api_base) else {
            return headers;
        };
        match loaded {
            LoadedSearch::Linkup(config) => config.validate_environment(headers, api_key, api_base),
            LoadedSearch::Searxng(config) => {
                config.validate_environment(headers, api_key, api_base)
            }
        }
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        optional_params: &Object,
        _body: Option<&SearchBody>,
        api_key: Option<&SecretString>,
    ) -> Result<String, SearchConfigError> {
        let loaded = self.reload(api_key, api_base)?;
        let stored = self.body.lock().unwrap_or_else(PoisonError::into_inner);
        match loaded {
            LoadedSearch::Linkup(config) => config
                .complete_url(api_base, optional_params, None, api_key)
                .map_err(|source| SearchConfigError::Provider(Box::new(source))),
            LoadedSearch::Searxng(config) => config
                .complete_url(api_base, optional_params, stored.as_ref(), api_key)
                .map_err(|source| SearchConfigError::Provider(Box::new(source))),
        }
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError> {
        match self.reload(input.api_key, input.api_base)? {
            LoadedSearch::Linkup(config) => {
                let typed = config
                    .transform_search_request(input)
                    .map_err(|source| SearchConfigError::Provider(Box::new(source)))?;
                encoded_body(&typed)
            }
            LoadedSearch::Searxng(config) => {
                let typed = config
                    .transform_search_request(input)
                    .map_err(|source| SearchConfigError::Provider(Box::new(source)))?;
                let body = encoded_body(&typed)?;
                *self.body.lock().unwrap_or_else(PoisonError::into_inner) = Some(typed);
                Ok(body)
            }
        }
    }
    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, SearchConfigError> {
        let text = response.text.as_bytes();
        match self.flavor {
            ReloadedFlavor::Linkup => decode_linkup_reply(text)
                .map_err(|source| SearchConfigError::Provider(Box::new(source))),
            ReloadedFlavor::Searxng => decode_searxng_reply(text)
                .map_err(|source| SearchConfigError::Provider(Box::new(source))),
        }
    }
}
