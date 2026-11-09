mod encoding;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::{CredentialError, ServerCredential};
use betterlite_openai::search::provider::{
    HttpMethod, PERPLEXITY_PARAMS, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use encoding::encode_params;

const KEY_VARIABLE: &str = "SEARXNG_API_KEY";
const BASE_VARIABLE: &str = "SEARXNG_API_BASE";
const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[KEY_VARIABLE],
    base_var: Some(BASE_VARIABLE),
    default_base: None,
};

#[derive(Debug, Error)]
pub enum SearxngSearchError {
    #[error(
        "SEARXNG_API_BASE is not set. Set the `SEARXNG_API_BASE` environment variable or pass an api_base"
    )]
    MissingApiBase,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the `{field}` search option has the wrong type")]
    OptionType { field: &'static str },
    #[error("the SearXNG search reply is not JSON: {source}")]
    ReplyJson {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearxngSearchRequest {
    #[serde(rename = "_searxng_params")]
    pub params: SearxngParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearxngParams {
    #[serde(rename = "q")]
    pub query: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Debug)]
pub struct SearxngSearchConfig {
    api_key: Option<SecretString>,
    environment_base: Option<String>,
}

impl SearxngSearchConfig {
    pub fn load(
        caller_key: Option<SecretString>,
        caller_base: Option<&str>,
        lookup: &dyn EnvironmentLookup,
    ) -> Result<Self, SearxngSearchError> {
        Ok(Self {
            api_key: CREDENTIAL.resolve(caller_key, caller_base, lookup)?,
            environment_base: lookup.lookup(BASE_VARIABLE).filter(|base| !base.is_empty()),
        })
    }
}

impl SearchProvider for SearxngSearchConfig {
    type Body = SearxngSearchRequest;
    type Error = SearxngSearchError;

    fn ui_friendly_name() -> &'static str {
        "SearXNG"
    }

    fn http_method(&self) -> HttpMethod {
        HttpMethod::Get
    }

    fn validate_environment(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Headers {
        let resolved = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(self.api_key.as_ref());
        if let Some(key) = resolved {
            headers.insert(
                "Authorization".to_owned(),
                format!("Bearer {}", key.expose_secret()),
            );
        }
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        headers
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        body: Option<&SearxngSearchRequest>,
    ) -> Result<String, SearxngSearchError> {
        search_url(api_base, self.environment_base.as_deref(), body)
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<SearxngSearchRequest, SearxngSearchError> {
        build_request(query, optional_params)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, SearxngSearchError> {
        decode_searxng_reply(response.text.as_bytes())
    }
}

pub fn search_url(
    api_base: Option<&str>,
    environment_base: Option<&str>,
    body: Option<&SearxngSearchRequest>,
) -> Result<String, SearxngSearchError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .or(environment_base)
        .ok_or(SearxngSearchError::MissingApiBase)?;
    let mut url = if base.ends_with("/search") {
        base.to_owned()
    } else if base.ends_with('/') {
        format!("{base}search")
    } else {
        format!("{base}/search")
    };
    if let Some(request) = body {
        url.push('?');
        url.push_str(&encode_params(&request.params));
    }
    Ok(url)
}

pub fn build_request(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<SearxngSearchRequest, SearxngSearchError> {
    let joined_query = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(items) => items.join(" "),
    };
    let language = match optional_params.get("country") {
        None => None,
        Some(country) if country.is_null() => None,
        Some(country) => match country.as_str() {
            Some(text) => Some(language_for(text)),
            None => {
                return Err(SearxngSearchError::OptionType { field: "country" });
            }
        },
    };
    let extra = passthrough(optional_params, language.is_some());
    Ok(SearxngSearchRequest {
        params: SearxngParams {
            query: joined_query,
            format: "json".to_owned(),
            language,
            extra,
        },
    })
}

fn language_for(country: &str) -> String {
    match country.to_lowercase().as_str() {
        "us" | "uk" => "en".to_owned(),
        "jp" => "ja".to_owned(),
        other => other.to_owned(),
    }
}

fn passthrough(optional_params: &Object, has_language: bool) -> Object {
    let mut extra = Object::new();
    for (name, value) in optional_params {
        if PERPLEXITY_PARAMS.contains(&name.as_str()) || is_body_key(name, has_language) {
            continue;
        }
        extra.insert(name.clone(), value.clone());
    }
    extra
}

fn is_body_key(name: &str, has_language: bool) -> bool {
    match name {
        "q" | "format" => true,
        "language" => has_language,
        _ => false,
    }
}

pub fn decode_searxng_reply(bytes: &[u8]) -> Result<SearchResponse, SearxngSearchError> {
    let reply: SearxngReply =
        serde_json::from_slice(bytes).map_err(|source| SearxngSearchError::ReplyJson { source })?;
    Ok(SearchResponse {
        results: reply.results.into_iter().map(project_item).collect(),
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

#[derive(Debug, Deserialize)]
struct SearxngReply {
    #[serde(default)]
    results: Vec<SearxngItem>,
}

#[derive(Debug, Deserialize)]
struct SearxngItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
    #[serde(default, rename = "publishedDate")]
    published_date: Option<String>,
    #[serde(default)]
    pubdate: Option<String>,
}

fn project_item(item: SearxngItem) -> SearchResult {
    SearchResult {
        title: item.title,
        url: item.url,
        snippet: item.content,
        date: item.published_date.or(item.pubdate),
        last_updated: None,
        extra: Object::new(),
    }
}

mod tests;
