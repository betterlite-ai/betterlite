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
        match self.flavor {
            ReloadedFlavor::Linkup => {
                let Ok(config) =
                    LinkupSearchConfig::load(api_key.cloned(), api_base, self.environment)
                else {
                    return headers;
                };
                config.validate_environment(headers, api_key, api_base)
            }
            ReloadedFlavor::Searxng => {
                let Ok(config) =
                    SearxngSearchConfig::load(api_key.cloned(), api_base, self.environment)
                else {
                    return headers;
                };
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
        match self.flavor {
            ReloadedFlavor::Linkup => {
                let config = LinkupSearchConfig::load(api_key.cloned(), api_base, self.environment)
                    .map_err(SearchConfigError::provider)?;
                config
                    .complete_url(api_base, optional_params, None, api_key)
                    .map_err(SearchConfigError::provider)
            }
            ReloadedFlavor::Searxng => {
                let config =
                    SearxngSearchConfig::load(api_key.cloned(), api_base, self.environment)
                        .map_err(SearchConfigError::provider)?;
                let stored = self.body.lock().unwrap_or_else(PoisonError::into_inner);
                config
                    .complete_url(api_base, optional_params, stored.as_ref(), api_key)
                    .map_err(SearchConfigError::provider)
            }
        }
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError> {
        match self.flavor {
            ReloadedFlavor::Linkup => {
                let typed = LinkupSearchConfig::load(
                    input.api_key.cloned(),
                    input.api_base,
                    self.environment,
                )
                .map_err(SearchConfigError::provider)?
                .transform_search_request(input)
                .map_err(SearchConfigError::provider)?;
                encoded_body(&typed)
            }
            ReloadedFlavor::Searxng => {
                let typed = SearxngSearchConfig::load(
                    input.api_key.cloned(),
                    input.api_base,
                    self.environment,
                )
                .map_err(SearchConfigError::provider)?
                .transform_search_request(input)
                .map_err(SearchConfigError::provider)?;
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
        match self.flavor {
            ReloadedFlavor::Linkup => {
                decode_linkup_reply(response.text.as_bytes()).map_err(SearchConfigError::provider)
            }
            ReloadedFlavor::Searxng => {
                decode_searxng_reply(response.text.as_bytes()).map_err(SearchConfigError::provider)
            }
        }
    }
}
