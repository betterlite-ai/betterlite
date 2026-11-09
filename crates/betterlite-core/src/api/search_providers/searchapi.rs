use std::fmt;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::{CredentialError, ServerCredential};
use betterlite_openai::search::provider::{HttpMethod, PERPLEXITY_PARAMS, SearchQuery};
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::google_pse::wire::{self, SearchParam, WireError};

const DEFAULT_API_BASE: &str = "https://www.searchapi.io/api/v1/search";
const API_KEY_VAR: &str = "SEARCHAPI_API_KEY";
const API_BASE_VAR: &str = "SEARCHAPI_API_BASE";
const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[API_KEY_VAR],
    base_var: Some(API_BASE_VAR),
    default_base: Some(DEFAULT_API_BASE),
};

#[derive(Debug, Error)]
pub enum SearchApiError {
    #[error("SEARCHAPI_API_KEY is not set. Set `SEARCHAPI_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("invalid SearchAPI.io response: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

pub struct SearchApiSearch<'env> {
    pub lookup: &'env dyn EnvironmentLookup,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

impl fmt::Debug for SearchApiSearch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchApiSearch")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("api_base", &self.api_base)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchApiBody {
    #[serde(rename = "_searchapi_params")]
    pub params: IndexMap<String, SearchParam>,
}

impl fmt::Debug for SearchApiBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        wire::debug_body(f, "SearchApiBody", "api_key", &self.params)
    }
}

#[derive(Debug, Deserialize)]
struct SearchApiReply {
    #[serde(default)]
    organic_results: Vec<SearchApiResult>,
}

#[derive(Debug, Deserialize)]
struct SearchApiResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    link: String,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    date: Option<String>,
}

impl SearchApiSearch<'_> {
    pub const UI_FRIENDLY_NAME: &'static str = "SearchAPI.io (Google Search)";
    pub const HTTP_METHOD: HttpMethod = HttpMethod::Get;

    pub fn validate_environment(&self, mut headers: Headers) -> Result<Headers, SearchApiError> {
        let resolved =
            CREDENTIAL.resolve(self.api_key.clone(), self.api_base.as_deref(), self.lookup)?;
        if resolved.is_none() {
            return Err(SearchApiError::MissingApiKey);
        }
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        Ok(headers)
    }

    pub fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<SearchApiBody, SearchApiError> {
        let api_key = CREDENTIAL
            .resolve(self.api_key.clone(), self.api_base.as_deref(), self.lookup)?
            .ok_or(SearchApiError::MissingApiKey)?;
        let query_text = join_query(query);
        let mut params = IndexMap::new();
        params.insert("engine".to_owned(), SearchParam::Text("google".to_owned()));
        params.insert("q".to_owned(), SearchParam::Text(query_text.clone()));
        params.insert(
            "api_key".to_owned(),
            SearchParam::Text(api_key.expose_secret().to_owned()),
        );
        if let Some(value) = optional_params.get("max_results") {
            let number: i64 =
                serde_json::from_value(value.clone()).map_err(|source| WireError::WrongType {
                    field: "max_results".to_owned(),
                    source,
                })?;
            params.insert("num".to_owned(), SearchParam::Number(number.min(10)));
        }
        if let Some(domains) = nonempty_domains(optional_params)? {
            params.insert(
                "q".to_owned(),
                SearchParam::Text(site_clause(&query_text, domains.as_slice())),
            );
        }
        if let Some(value) = optional_params.get("country") {
            let country: String =
                serde_json::from_value(value.clone()).map_err(|source| WireError::WrongType {
                    field: "country".to_owned(),
                    source,
                })?;
            params.insert("gl".to_owned(), SearchParam::Text(country.to_lowercase()));
        }
        copy_passthrough_params(optional_params, &mut params)?;
        Ok(SearchApiBody { params })
    }

    pub fn complete_url(
        &self,
        api_base: Option<&str>,
        body: Option<&SearchApiBody>,
    ) -> Result<String, SearchApiError> {
        let base = api_base
            .filter(|base| !base.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                self.lookup
                    .lookup(API_BASE_VAR)
                    .filter(|base| !base.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        match body {
            Some(record) => Ok(format!(
                "{base}?{}",
                wire::query_string(&record.params, true)?
            )),
            None => Ok(base),
        }
    }
}

pub fn decode_reply(text: &str) -> Result<SearchResponse, SearchApiError> {
    let reply: SearchApiReply =
        serde_json::from_str(text).map_err(|source| SearchApiError::Decode { source })?;
    let results = reply
        .organic_results
        .into_iter()
        .map(project_result)
        .collect();
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn project_result(result: SearchApiResult) -> SearchResult {
    SearchResult {
        title: result.title,
        url: result.link,
        snippet: result.snippet,
        date: result.date,
        last_updated: None,
        extra: Object::new(),
    }
}

fn join_query(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(items) => items.join(" "),
    }
}

fn nonempty_domains(optional_params: &Object) -> Result<Option<Vec<String>>, WireError> {
    let Some(value) = optional_params.get("search_domain_filter") else {
        return Ok(None);
    };
    let Some(domains) = value.as_array() else {
        return Ok(None);
    };
    if domains.is_empty() {
        return Ok(None);
    }
    let mut decoded = Vec::with_capacity(domains.len());
    for domain in domains {
        let text: String =
            serde_json::from_value(domain.clone()).map_err(|source| WireError::WrongType {
                field: "search_domain_filter".to_owned(),
                source,
            })?;
        decoded.push(text);
    }
    Ok(Some(decoded))
}

fn site_clause(query: &str, domains: &[String]) -> String {
    let clauses: Vec<String> = domains
        .iter()
        .map(|domain| format!("site:{domain}"))
        .collect();
    format!("({query}) AND ({})", clauses.join(" OR "))
}

fn copy_passthrough_params(
    optional_params: &Object,
    params: &mut IndexMap<String, SearchParam>,
) -> Result<(), WireError> {
    for (name, value) in optional_params {
        if PERPLEXITY_PARAMS.contains(&name.as_str()) || params.contains_key(name) {
            continue;
        }
        let param: SearchParam =
            serde_json::from_value(value.clone()).map_err(|source| WireError::WrongType {
                field: name.clone(),
                source,
            })?;
        params.insert(name.clone(), param);
    }
    Ok(())
}

mod tests;
