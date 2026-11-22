use super::super::config::{SearchBody, SearchHooks};
use super::super::error::SearchConfigError;
use super::shared::encoded_body;
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
// Google PSE and SearchAPI keep their key in the URL, so only a fetch's query string comes from the stored body
pub(super) struct UrlKeyedProvider<'env> {
    pub(super) environment: &'env dyn EnvironmentLookup,
    pub(super) flavor: Mutex<UrlKeyedFlavor>,
}
pub(super) enum UrlKeyedFlavor {
    GooglePse(Option<GooglePseBody>),
    SearchApi(Option<SearchApiBody>),
}
impl SearchHooks for UrlKeyedProvider<'_> {
    fn http_method(&self) -> HttpMethod {
        HttpMethod::Get
    }
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        let flavor = self.flavor.lock().unwrap_or_else(PoisonError::into_inner);
        match &*flavor {
            UrlKeyedFlavor::GooglePse(_) => GooglePseSearch {
                lookup: self.environment,
                api_key: api_key.cloned(),
                api_base: api_base.map(str::to_owned),
                search_engine_id: None,
            }
            .validate_environment(headers.clone())
            .unwrap_or(headers),
            UrlKeyedFlavor::SearchApi(_) => SearchApiSearch {
                lookup: self.environment,
                api_key: api_key.cloned(),
                api_base: api_base.map(str::to_owned),
            }
            .validate_environment(headers.clone())
            .unwrap_or(headers),
        }
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&SearchBody>,
        api_key: Option<&SecretString>,
    ) -> Result<String, SearchConfigError> {
        let flavor = self.flavor.lock().unwrap_or_else(PoisonError::into_inner);
        match &*flavor {
            UrlKeyedFlavor::GooglePse(stored) => GooglePseSearch {
                lookup: self.environment,
                api_key: api_key.cloned(),
                api_base: api_base.map(str::to_owned),
                search_engine_id: None,
            }
            .complete_url(api_base, stored.as_ref())
            .map_err(SearchConfigError::provider),
            UrlKeyedFlavor::SearchApi(stored) => SearchApiSearch {
                lookup: self.environment,
                api_key: api_key.cloned(),
                api_base: api_base.map(str::to_owned),
            }
            .complete_url(api_base, stored.as_ref())
            .map_err(SearchConfigError::provider),
        }
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError> {
        let mut flavor = self.flavor.lock().unwrap_or_else(PoisonError::into_inner);
        match &mut *flavor {
            UrlKeyedFlavor::GooglePse(stored) => {
                let typed = GooglePseSearch {
                    lookup: self.environment,
                    api_key: input.api_key.cloned(),
                    api_base: input.api_base.map(str::to_owned),
                    search_engine_id: None,
                }
                .transform_search_request(input.query, input.optional_params)
                .map_err(SearchConfigError::provider)?;
                let body = encoded_body(&typed)?;
                *stored = Some(typed);
                Ok(body)
            }
            UrlKeyedFlavor::SearchApi(stored) => {
                let typed = SearchApiSearch {
                    lookup: self.environment,
                    api_key: input.api_key.cloned(),
                    api_base: input.api_base.map(str::to_owned),
                }
                .transform_search_request(input.query, input.optional_params)
                .map_err(SearchConfigError::provider)?;
                let body = encoded_body(&typed)?;
                *stored = Some(typed);
                Ok(body)
            }
        }
    }
    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, SearchConfigError> {
        let flavor = self.flavor.lock().unwrap_or_else(PoisonError::into_inner);
        match &*flavor {
            UrlKeyedFlavor::GooglePse(_) => {
                decode_google_pse_reply(response.text).map_err(SearchConfigError::provider)
            }
            UrlKeyedFlavor::SearchApi(_) => {
                decode_searchapi_reply(response.text).map_err(SearchConfigError::provider)
            }
        }
    }
}
