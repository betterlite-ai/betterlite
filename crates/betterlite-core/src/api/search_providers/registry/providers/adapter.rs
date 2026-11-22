use super::super::config::{SearchBody, SearchHooks};
use super::super::error::SearchConfigError;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, RequestInput, SearchProvider,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::SecretString;
use std::sync::{Mutex, PoisonError};
// a provider whose config keeps one stable load: the adapter forwards every hook and keeps the typed request body
pub(super) struct Adapter<P: SearchProvider> {
    provider: P,
    body: Mutex<Option<P::Body>>,
}
impl<P: SearchProvider> Adapter<P> {
    pub(super) const fn new(provider: P) -> Self {
        Self {
            provider,
            body: Mutex::new(None),
        }
    }
}
impl<P> SearchHooks for Adapter<P>
where
    P: SearchProvider + Send + Sync,
    P::Body: Send + 'static,
    P::Error: Send + Sync + 'static,
{
    fn http_method(&self) -> HttpMethod {
        self.provider.http_method()
    }
    fn supports_rich_search_input(&self) -> bool {
        self.provider.supports_rich_search_input()
    }
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        self.provider
            .validate_environment(headers, api_key, api_base)
    }
    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, SearchBody>,
    ) -> Result<(Headers, Option<Vec<u8>>), SearchConfigError> {
        let stored = self.body.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(body) = stored.as_ref() else {
            return Err(SearchConfigError::MissingBody);
        };
        self.provider
            .sign_request(
                headers,
                &OutboundSearch {
                    api_base: request.api_base,
                    api_key: request.api_key,
                    optional_params: request.optional_params,
                    body,
                },
            )
            .map_err(|source| SearchConfigError::Provider(Box::new(source)))
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        optional_params: &Object,
        _body: Option<&SearchBody>,
        api_key: Option<&SecretString>,
    ) -> Result<String, SearchConfigError> {
        let stored = self.body.lock().unwrap_or_else(PoisonError::into_inner);
        self.provider
            .complete_url(api_base, optional_params, stored.as_ref(), api_key)
            .map_err(|source| SearchConfigError::Provider(Box::new(source)))
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError> {
        let typed = self
            .provider
            .transform_search_request(input)
            .map_err(|source| SearchConfigError::Provider(Box::new(source)))?;
        let encoded =
            serde_json::to_value(&typed).map_err(|source| SearchConfigError::Encode { source })?;
        *self.body.lock().unwrap_or_else(PoisonError::into_inner) = Some(typed);
        Ok(SearchBody(encoded))
    }
    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, SearchConfigError> {
        self.provider
            .transform_search_response(response)
            .map_err(|source| SearchConfigError::Provider(Box::new(source)))
    }
    fn http_error(&self, response: &ProviderResponse<'_>) -> ProviderError {
        self.provider.http_error(response)
    }
    fn error_class(&self, message: &str, status_code: u16, headers: &Headers) -> ProviderError {
        self.provider.error_class(message, status_code, headers)
    }
}
