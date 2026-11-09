pub mod wire;

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

use self::wire::{SearchParam, WireError};

const DEFAULT_API_BASE: &str = "https://www.googleapis.com/customsearch/v1";
const API_KEY_VAR: &str = "GOOGLE_PSE_API_KEY";
const API_BASE_VAR: &str = "GOOGLE_PSE_API_BASE";
const ENGINE_ID_VAR: &str = "GOOGLE_PSE_ENGINE_ID";
const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[API_KEY_VAR],
    base_var: Some(API_BASE_VAR),
    default_base: Some(DEFAULT_API_BASE),
};

#[derive(Debug, Error)]
pub enum GooglePseError {
    #[error("GOOGLE_PSE_API_KEY is not set. Set `GOOGLE_PSE_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(
        "GOOGLE_PSE_ENGINE_ID is not set. Set `GOOGLE_PSE_ENGINE_ID` environment variable or pass `search_engine_id` parameter."
    )]
    MissingEngineId,
    #[error("GOOGLE_PSE_API_KEY is required")]
    ApiKeyRequired,
    #[error("GOOGLE_PSE_ENGINE_ID is required")]
    EngineIdRequired,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("invalid Google PSE response: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

pub struct GooglePseSearch<'env> {
    pub lookup: &'env dyn EnvironmentLookup,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub search_engine_id: Option<String>,
}

impl fmt::Debug for GooglePseSearch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GooglePseSearch")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("api_base", &self.api_base)
            .field("search_engine_id", &self.search_engine_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GooglePseBody {
    #[serde(rename = "_google_pse_params")]
    pub params: IndexMap<String, SearchParam>,
}

impl fmt::Debug for GooglePseBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        wire::debug_body(f, "GooglePseBody", "key", &self.params)
    }
}

#[derive(Debug, Deserialize)]
struct GooglePseReply {
    #[serde(default)]
    items: Vec<GooglePseItem>,
}

#[derive(Debug, Deserialize)]
struct GooglePseItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    link: String,
    #[serde(default)]
    snippet: String,
}

impl GooglePseSearch<'_> {
    pub const UI_FRIENDLY_NAME: &'static str = "Google PSE";
    pub const HTTP_METHOD: HttpMethod = HttpMethod::Get;

    pub fn validate_environment(&self, mut headers: Headers) -> Result<Headers, GooglePseError> {
        let resolved =
            CREDENTIAL.resolve(self.api_key.clone(), self.api_base.as_deref(), self.lookup)?;
        if resolved.is_none() {
            return Err(GooglePseError::MissingApiKey);
        }
        if self.resolve_engine_id().is_none() {
            return Err(GooglePseError::MissingEngineId);
        }
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        Ok(headers)
    }

    pub fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<GooglePseBody, GooglePseError> {
        let api_key = CREDENTIAL
            .resolve(self.api_key.clone(), self.api_base.as_deref(), self.lookup)?
            .ok_or(GooglePseError::ApiKeyRequired)?;
        let engine_id = self
            .resolve_engine_id()
            .ok_or(GooglePseError::EngineIdRequired)?;
        let mut params = IndexMap::new();
        params.insert("q".to_owned(), SearchParam::Text(join_query(query)));
        params.insert("cx".to_owned(), SearchParam::Text(engine_id));
        params.insert(
            "key".to_owned(),
            SearchParam::Text(api_key.expose_secret().to_owned()),
        );
        copy_unified_params(optional_params, &mut params)?;
        copy_passthrough_params(optional_params, &mut params)?;
        Ok(GooglePseBody { params })
    }

    pub fn complete_url(
        &self,
        api_base: Option<&str>,
        body: Option<&GooglePseBody>,
    ) -> Result<String, GooglePseError> {
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
                wire::query_string(&record.params, false)?
            )),
            None => Ok(base),
        }
    }

    fn resolve_engine_id(&self) -> Option<String> {
        self.search_engine_id
            .clone()
            .filter(|engine_id| !engine_id.is_empty())
            .or_else(|| {
                self.lookup
                    .lookup(ENGINE_ID_VAR)
                    .filter(|engine_id| !engine_id.is_empty())
            })
    }
}

pub fn decode_reply(text: &str) -> Result<SearchResponse, GooglePseError> {
    let reply: GooglePseReply =
        serde_json::from_str(text).map_err(|source| GooglePseError::Decode { source })?;
    let results = reply.items.into_iter().map(project_item).collect();
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn project_item(item: GooglePseItem) -> SearchResult {
    SearchResult {
        title: item.title,
        url: item.link,
        snippet: item.snippet,
        date: None,
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

fn copy_unified_params(
    optional_params: &Object,
    params: &mut IndexMap<String, SearchParam>,
) -> Result<(), WireError> {
    if let Some(value) = optional_params.get("max_results") {
        let number: i64 =
            serde_json::from_value(value.clone()).map_err(|source| WireError::WrongType {
                field: "max_results".to_owned(),
                source,
            })?;
        params.insert("num".to_owned(), SearchParam::Number(number.min(10)));
    }
    if let Some(domain) = first_domain(optional_params)? {
        params.insert("siteSearch".to_owned(), SearchParam::Text(domain));
        params.insert(
            "siteSearchFilter".to_owned(),
            SearchParam::Text("i".to_owned()),
        );
    }
    if let Some(value) = optional_params.get("country") {
        let country: String =
            serde_json::from_value(value.clone()).map_err(|source| WireError::WrongType {
                field: "country".to_owned(),
                source,
            })?;
        params.insert("gl".to_owned(), SearchParam::Text(country.to_uppercase()));
    }
    Ok(())
}

fn first_domain(optional_params: &Object) -> Result<Option<String>, WireError> {
    let Some(value) = optional_params.get("search_domain_filter") else {
        return Ok(None);
    };
    let Some(domains) = value.as_array() else {
        return Ok(value.as_str().map(str::to_owned));
    };
    let Some(first) = domains.first() else {
        return Ok(None);
    };
    let domain: String =
        serde_json::from_value(first.clone()).map_err(|source| WireError::WrongType {
            field: "search_domain_filter".to_owned(),
            source,
        })?;
    Ok(Some(domain))
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
