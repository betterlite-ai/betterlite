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

use error::ParallelAiSearchError;
use reply::decode_parallel_ai_reply;
use request::{SearchBody, search_body};

pub const PARALLEL_AI_API_BASE: &str = "https://api.parallel.ai";
const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &["PARALLEL_AI_API_KEY", "PARALLEL_API_KEY"],
    base_var: Some("PARALLEL_AI_API_BASE"),
    default_base: Some(PARALLEL_AI_API_BASE),
};

#[derive(Debug, Clone)]
pub struct ParallelAiSearch {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl ParallelAiSearch {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: ["PARALLEL_AI_API_KEY", "PARALLEL_API_KEY"]
                .iter()
                .find_map(|name| present(*name))
                .map(SecretString::from),
            api_base: present("PARALLEL_AI_API_BASE")
                .unwrap_or_else(|| PARALLEL_AI_API_BASE.to_owned()),
        }
    }

    #[must_use]
    pub fn endpoint(&self, api_base: Option<&str>) -> String {
        let trimmed = api_base.unwrap_or(&self.api_base).trim_end_matches('/');
        if trimmed.ends_with("/v1/search") {
            trimmed.to_owned()
        } else {
            format!(
                "{}/v1/search",
                trimmed.strip_suffix("/v1").unwrap_or(trimmed)
            )
        }
    }

    pub fn prepare_headers(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, ParallelAiSearchError> {
        let environment = ParallelAiEnvironment {
            api_key: self.api_key.as_ref(),
            api_base: &self.api_base,
        };
        let Some(resolved) = CREDENTIAL.resolve(api_key.cloned(), api_base, &environment)? else {
            return Err(ParallelAiSearchError::MissingApiKey);
        };
        Ok(auth_headers(headers, &resolved))
    }
}

fn auth_headers(mut headers: Headers, key: &SecretString) -> Headers {
    headers.insert("x-api-key".to_owned(), key.expose_secret().to_owned());
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    headers
}

struct ParallelAiEnvironment<'settings> {
    api_key: Option<&'settings SecretString>,
    api_base: &'settings str,
}

impl EnvironmentLookup for ParallelAiEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            "PARALLEL_AI_API_KEY" | "PARALLEL_API_KEY" => {
                self.api_key.map(|key| key.expose_secret().to_owned())
            }
            "PARALLEL_AI_API_BASE" => Some(self.api_base.to_owned()),
            _ => None,
        }
    }
}

impl SearchProvider for ParallelAiSearch {
    type Body = SearchBody;
    type Error = ParallelAiSearchError;

    fn ui_friendly_name() -> &'static str {
        "Parallel AI"
    }

    fn supports_rich_search_input(&self) -> bool {
        true
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

    // LiteLLM raises the credential and host checks from validate_environment, which cannot fail here
    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), ParallelAiSearchError> {
        let signed = self.prepare_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((signed, None))
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&SearchBody>,
    ) -> Result<String, ParallelAiSearchError> {
        Ok(self.endpoint(api_base))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<SearchBody, ParallelAiSearchError> {
        search_body(query, optional_params)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, ParallelAiSearchError> {
        Ok(decode_parallel_ai_reply(response.text.as_bytes())?)
    }
}
