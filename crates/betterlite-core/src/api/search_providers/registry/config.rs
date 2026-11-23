use super::error::SearchConfigError;
use super::names::SearchProviderName;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, RequestInput, SearchProvider,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
// the one body type every provider's request becomes, so a caller can hold any of them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SearchBody(pub JsonValue);
pub struct SearchConfig<'env> {
    pub name: SearchProviderName,
    pub hooks: Box<dyn SearchHooks + 'env>,
}
impl SearchProvider for SearchConfig<'_> {
    type Body = SearchBody;
    type Error = SearchConfigError;
    fn http_method(&self) -> HttpMethod {
        self.hooks.http_method()
    }
    fn supports_rich_search_input(&self) -> bool {
        self.hooks.supports_rich_search_input()
    }
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        self.hooks.validate_environment(headers, api_key, api_base)
    }
    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, SearchBody>,
    ) -> Result<(Headers, Option<Vec<u8>>), SearchConfigError> {
        self.hooks.sign_request(headers, request)
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        optional_params: &Object,
        body: Option<&SearchBody>,
        api_key: Option<&SecretString>,
    ) -> Result<String, SearchConfigError> {
        self.hooks
            .complete_url(api_base, optional_params, body, api_key)
    }
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError> {
        self.hooks.transform_search_request(input)
    }
    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, SearchConfigError> {
        self.hooks.transform_search_response(response)
    }
    fn http_error(&self, response: &ProviderResponse<'_>) -> ProviderError {
        self.hooks.http_error(response)
    }
    fn error_class(&self, message: &str, status_code: u16, headers: &Headers) -> ProviderError {
        self.hooks.error_class(message, status_code, headers)
    }
}
// B3270's hooks with the body type erased, each provider struct adapting itself to them
pub trait SearchHooks: Send + Sync {
    fn http_method(&self) -> HttpMethod {
        HttpMethod::Post
    }
    fn supports_rich_search_input(&self) -> bool {
        false
    }
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers;
    fn sign_request(
        &self,
        headers: Headers,
        _request: &OutboundSearch<'_, SearchBody>,
    ) -> Result<(Headers, Option<Vec<u8>>), SearchConfigError> {
        Ok((headers, None))
    }
    fn complete_url(
        &self,
        api_base: Option<&str>,
        optional_params: &Object,
        body: Option<&SearchBody>,
        api_key: Option<&SecretString>,
    ) -> Result<String, SearchConfigError>;
    fn transform_search_request(
        &self,
        input: &RequestInput<'_>,
    ) -> Result<SearchBody, SearchConfigError>;
    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, SearchConfigError>;
    fn http_error(&self, response: &ProviderResponse<'_>) -> ProviderError {
        self.error_class(response.text, response.status_code, response.headers)
    }
    fn error_class(&self, message: &str, status_code: u16, headers: &Headers) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: message.to_owned(),
            headers: Some(headers.clone().into_iter().collect()),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}
