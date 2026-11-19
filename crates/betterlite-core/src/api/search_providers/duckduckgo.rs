use std::sync::{Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    HttpMethod, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::SecretString;
use thiserror::Error;

pub mod reply;
pub mod request;

use request::{DuckDuckGoParams, DuckDuckGoSearchRequest};

const DEFAULT_API_BASE: &str = "https://api.duckduckgo.com";
const API_BASE_VARIABLE: &str = "DUCKDUCKGO_API_BASE";

#[derive(Debug, Error)]
pub enum DuckDuckGoSearchError {
    #[error("the `{field}` search option has the wrong type")]
    OptionType { field: &'static str },
    #[error("the DuckDuckGo search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the DuckDuckGo search reply is not a JSON object")]
    ReplyNotObject,
    #[error("the DuckDuckGo search reply is not JSON: {source}")]
    ReplyJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the DuckDuckGo search reply field `{field}` is not text")]
    ReplyText {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the DuckDuckGo search reply field `{field}` is not a JSON list")]
    ReplyList {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the DuckDuckGo search reply field `{field}` is null")]
    ReplyNull { field: &'static str },
    #[error("a nested DuckDuckGo RelatedTopics entry is not a JSON object")]
    NestedEntry,
}

// the response hook takes only &self, so an unsent request reads as an empty query with no limit
const NO_REQUEST: DuckDuckGoParams = DuckDuckGoParams {
    query: String::new(),
    format: String::new(),
    max_results: None,
    pretty: None,
    no_redirect: None,
    no_html: None,
    skip_disambig: None,
};

#[derive(Debug)]
pub struct DuckDuckGoSearchConfig {
    environment_base: String,
    request_params: Mutex<DuckDuckGoParams>,
}

impl DuckDuckGoSearchConfig {
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        Self {
            environment_base: lookup
                .lookup(API_BASE_VARIABLE)
                .filter(|base| !base.is_empty())
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
            request_params: Mutex::new(NO_REQUEST),
        }
    }
}

impl SearchProvider for DuckDuckGoSearchConfig {
    type Body = DuckDuckGoSearchRequest;
    type Error = DuckDuckGoSearchError;

    fn ui_friendly_name() -> &'static str {
        "DuckDuckGo"
    }

    fn http_method(&self) -> HttpMethod {
        HttpMethod::Get
    }

    fn validate_environment(
        &self,
        mut headers: Headers,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Headers {
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        headers
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        body: Option<&DuckDuckGoSearchRequest>,
    ) -> Result<String, DuckDuckGoSearchError> {
        Ok(request::search_url(api_base, &self.environment_base, body))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<DuckDuckGoSearchRequest, DuckDuckGoSearchError> {
        let body = request::build_request(query, optional_params)?;
        // the response hook gets only &self, so the config keeps the query its reply must decode with
        *self
            .request_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = body.params.clone();
        Ok(body)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, DuckDuckGoSearchError> {
        let params = self
            .request_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        reply::decode_duckduckgo_search(response.text.as_bytes(), &params)
    }
}

mod tests;
