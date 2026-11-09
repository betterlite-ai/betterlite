mod credentials;
mod endpoint;
pub mod error;
pub mod reply;
mod request;
mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::ServerCredential;
use betterlite_openai::search::provider::{
    OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret as _, SecretString};

use credentials::compose_headers;
use error::YouComError;
use reply::decode_you_com_reply;
use request::search_body;

pub const YOU_COM_API_BASE: &str = "https://ydc-index.io";
pub const YOU_COM_FREE_API_BASE: &str = "https://api.you.com/v1/agents/search";
pub const YOU_COM_CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &["YOUCOM_API_KEY"],
    base_var: Some("YOUCOM_API_BASE"),
    default_base: Some(YOU_COM_API_BASE),
};

#[derive(Debug, Clone)]
pub struct YouComSearch {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

impl YouComSearch {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present("YOUCOM_API_KEY").map(SecretString::from),
            api_base: present("YOUCOM_API_BASE"),
        }
    }

    #[must_use]
    pub fn endpoint(&self, api_base: Option<&str>, api_key: Option<&SecretString>) -> String {
        endpoint::endpoint(self, api_base, api_key)
    }

    pub fn resolve_api_key(
        &self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Option<SecretString>, YouComError> {
        credentials::resolve(self, api_key, api_base)
    }

    pub fn prepare_headers(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, YouComError> {
        let credential = self.resolve_api_key(api_key, api_base)?;
        Ok(compose_headers(headers, credential.as_ref()))
    }
}

impl SearchProvider for YouComSearch {
    type Body = Object;
    type Error = YouComError;

    fn ui_friendly_name() -> &'static str {
        "You.com"
    }

    // the trait hook cannot fail, so it shapes headers only and sign_request enforces the key policy
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Headers {
        let resolved = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(self.api_key.as_ref());
        compose_headers(headers, resolved)
    }

    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), YouComError> {
        let validated = self.prepare_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((validated, None))
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&Object>,
    ) -> Result<String, YouComError> {
        Ok(self.endpoint(api_base, None))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<Object, YouComError> {
        search_body(query, optional_params)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, YouComError> {
        decode_you_com_reply(response.text.as_bytes())
    }
}
