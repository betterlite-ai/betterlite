pub mod error;
pub mod reply;
pub mod request;
mod tests;

use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::ServerCredential;
use betterlite_openai::search::provider::{
    OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret as _, SecretString};

use error::FastcrwError;
use reply::decode_fastcrw_reply;
use request::{SearchBody, search_body};

pub const FASTCRW_API_BASE: &str = "https://fastcrw.com/api/v1";
pub const FASTCRW_CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &["CRW_API_KEY"],
    base_var: Some("CRW_API_BASE"),
    default_base: Some(FASTCRW_API_BASE),
};

#[derive(Debug, Clone)]
pub struct FastcrwSearch {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl FastcrwSearch {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present("CRW_API_KEY").map(SecretString::from),
            api_base: present("CRW_API_BASE").unwrap_or_else(|| FASTCRW_API_BASE.to_owned()),
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
    ) -> Result<Headers, FastcrwError> {
        let mut environment = BTreeMap::new();
        if let Some(key) = &self.api_key {
            environment.insert("CRW_API_KEY".to_owned(), key.expose_secret().to_owned());
        }
        environment.insert("CRW_API_BASE".to_owned(), self.api_base.clone());
        let resolved = FASTCRW_CREDENTIAL
            .resolve(api_key.cloned(), api_base, &environment)?
            .ok_or(FastcrwError::MissingApiKey)?;
        Ok(auth_headers(headers, &resolved))
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

impl SearchProvider for FastcrwSearch {
    type Body = SearchBody;
    type Error = FastcrwError;

    fn ui_friendly_name() -> &'static str {
        "fastCRW"
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
    ) -> Result<String, FastcrwError> {
        Ok(self.endpoint(api_base))
    }

    // LiteLLM raises the credential and host checks from validate_environment, which cannot fail here
    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), FastcrwError> {
        let signed = self.prepare_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((signed, None))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<SearchBody, FastcrwError> {
        Ok(search_body(query, optional_params))
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, FastcrwError> {
        decode_fastcrw_reply(response.text.as_bytes())
    }
}
