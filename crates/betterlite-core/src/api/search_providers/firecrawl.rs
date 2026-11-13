pub mod error;
pub mod reply;
pub mod request;
mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::ServerCredential;
use betterlite_openai::search::provider::{
    OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret as _, SecretString};

use error::FirecrawlError;
use reply::decode_firecrawl_reply;
use request::{SearchBody, search_body};

pub const FIRECRAWL_API_BASE: &str = "https://api.firecrawl.dev/v2";
pub const FIRECRAWL_CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &["FIRECRAWL_API_KEY"],
    base_var: Some("FIRECRAWL_API_BASE"),
    default_base: Some(FIRECRAWL_API_BASE),
};

#[derive(Debug, Clone)]
pub struct FirecrawlSearch {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl FirecrawlSearch {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present("FIRECRAWL_API_KEY").map(SecretString::from),
            api_base: present("FIRECRAWL_API_BASE")
                .unwrap_or_else(|| FIRECRAWL_API_BASE.to_owned()),
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

    pub fn prepare_headers(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, FirecrawlError> {
        let environment = FirecrawlEnvironment {
            api_key: self.api_key.as_ref(),
            api_base: &self.api_base,
        };
        let resolved = FIRECRAWL_CREDENTIAL.resolve(api_key.cloned(), api_base, &environment)?;
        let Some(present) = resolved else {
            return Err(FirecrawlError::MissingApiKey);
        };
        Ok(auth_headers(headers, &present))
    }
}

fn auth_headers(mut headers: Headers, key: &SecretString) -> Headers {
    headers.insert(
        "Authorization".to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    headers
}

struct FirecrawlEnvironment<'settings> {
    api_key: Option<&'settings SecretString>,
    api_base: &'settings str,
}

impl EnvironmentLookup for FirecrawlEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            "FIRECRAWL_API_KEY" => self.api_key.map(|key| key.expose_secret().to_owned()),
            "FIRECRAWL_API_BASE" => Some(self.api_base.to_owned()),
            _ => None,
        }
    }
}

impl SearchProvider for FirecrawlSearch {
    type Body = SearchBody;
    type Error = FirecrawlError;

    fn ui_friendly_name() -> &'static str {
        "Firecrawl"
    }

    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Headers {
        let key = api_key
            .filter(|candidate| !candidate.expose_secret().is_empty())
            .or(self.api_key.as_ref());
        match key {
            Some(present) => auth_headers(headers, present),
            None => headers,
        }
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&SearchBody>,
    ) -> Result<String, FirecrawlError> {
        Ok(self.endpoint(api_base))
    }

    // LiteLLM raises the credential and host checks from validate_environment, which cannot fail here
    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), FirecrawlError> {
        let signed = self.prepare_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((signed, None))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<SearchBody, FirecrawlError> {
        Ok(search_body(query, optional_params))
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, FirecrawlError> {
        decode_firecrawl_reply(response.text.as_bytes())
    }
}
