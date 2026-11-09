mod credentials;
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

use credentials::{TavilyEnvironment, auth_headers};
use error::TavilyError;
use reply::decode_tavily_reply;
use request::search_body;

pub const TAVILY_API_BASE: &str = "https://api.tavily.com";
pub const TAVILY_CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &["TAVILY_API_KEY"],
    base_var: Some("TAVILY_API_BASE"),
    default_base: Some(TAVILY_API_BASE),
};

#[derive(Debug, Clone)]
pub struct TavilySearch {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl TavilySearch {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present("TAVILY_API_KEY").map(SecretString::from),
            api_base: present("TAVILY_API_BASE").unwrap_or_else(|| TAVILY_API_BASE.to_owned()),
        }
    }

    #[must_use]
    pub fn endpoint(&self, api_base: Option<&str>) -> String {
        let base = api_base.unwrap_or(&self.api_base);
        if base.ends_with("/search") {
            base.to_owned()
        } else {
            format!("{base}/search")
        }
    }

    pub fn resolve_api_key(
        &self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<SecretString, TavilyError> {
        let environment = TavilyEnvironment {
            api_key: self.api_key.as_ref(),
            api_base: &self.api_base,
        };
        TAVILY_CREDENTIAL
            .resolve(api_key.cloned(), api_base, &environment)?
            .ok_or(TavilyError::MissingApiKey)
    }

    pub fn prepare_headers(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, TavilyError> {
        let credential = self.resolve_api_key(api_key, api_base)?;
        let mut prepared = headers;
        prepared.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", credential.expose_secret()),
        );
        prepared.insert("Content-Type".to_owned(), "application/json".to_owned());
        Ok(prepared)
    }
}

impl SearchProvider for TavilySearch {
    type Body = Object;
    type Error = TavilyError;

    fn ui_friendly_name() -> &'static str {
        "Tavily"
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
        if let Some(credential) = resolved {
            return auth_headers(headers, credential);
        }
        headers
    }

    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), TavilyError> {
        let validated = self.prepare_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((validated, None))
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&Object>,
    ) -> Result<String, TavilyError> {
        Ok(self.endpoint(api_base))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<Object, TavilyError> {
        search_body(query, optional_params)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, TavilyError> {
        decode_tavily_reply(response.text.as_bytes())
    }
}
