use std::collections::BTreeSet;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::{CredentialError, ServerCredential};
use betterlite_openai::search::provider::{
    PERPLEXITY_PARAMS, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_API_BASE: &str = "https://api.linkup.so/v1";
const DEFAULT_DEPTH: &str = "standard";
const DEFAULT_OUTPUT_TYPE: &str = "searchResults";
const KEY_VARIABLE: &str = "LINKUP_API_KEY";
const BASE_VARIABLE: &str = "LINKUP_API_BASE";
const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[KEY_VARIABLE],
    base_var: Some(BASE_VARIABLE),
    default_base: Some(DEFAULT_API_BASE),
};

#[derive(Debug, Error)]
pub enum LinkupSearchError {
    #[error("LINKUP_API_KEY is not set. Set `LINKUP_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the `{field}` search option has the wrong type")]
    OptionType { field: &'static str },
    #[error("the Linkup search reply is not JSON: {source}")]
    ReplyJson {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkupSearchRequest {
    #[serde(rename = "q")]
    pub query: String,
    pub depth: String,
    #[serde(rename = "outputType")]
    pub output_type: String,
    #[serde(
        default,
        rename = "maxResults",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_results: Option<i64>,
    #[serde(
        default,
        rename = "includeDomains",
        skip_serializing_if = "Option::is_none"
    )]
    pub include_domains: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Debug)]
pub struct LinkupSearchConfig {
    api_key: SecretString,
    base: String,
}

impl LinkupSearchConfig {
    pub fn load(
        caller_key: Option<SecretString>,
        caller_base: Option<&str>,
        lookup: &dyn EnvironmentLookup,
    ) -> Result<Self, LinkupSearchError> {
        let Some(api_key) = CREDENTIAL.resolve(caller_key, caller_base, lookup)? else {
            return Err(LinkupSearchError::MissingApiKey);
        };
        Ok(Self {
            api_key,
            base: resolve_base(caller_base, lookup),
        })
    }
}

impl SearchProvider for LinkupSearchConfig {
    type Body = LinkupSearchRequest;
    type Error = LinkupSearchError;

    fn ui_friendly_name() -> &'static str {
        "Linkup"
    }

    fn validate_environment(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Headers {
        let key = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .unwrap_or(&self.api_key);
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        headers
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&LinkupSearchRequest>,
    ) -> Result<String, LinkupSearchError> {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(&self.base);
        Ok(search_endpoint(base))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<LinkupSearchRequest, LinkupSearchError> {
        build_request(query, optional_params)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, LinkupSearchError> {
        decode_linkup_reply(response.text.as_bytes())
    }
}

#[must_use]
pub fn search_endpoint(base: &str) -> String {
    if base.ends_with("/search") {
        base.to_owned()
    } else {
        format!("{base}/search")
    }
}

fn resolve_base(caller_base: Option<&str>, lookup: &dyn EnvironmentLookup) -> String {
    let environment_base = lookup.lookup(BASE_VARIABLE).filter(|base| !base.is_empty());
    caller_base
        .filter(|base| !base.is_empty())
        .or(environment_base.as_deref())
        .unwrap_or(DEFAULT_API_BASE)
        .to_owned()
}

pub fn build_request(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<LinkupSearchRequest, LinkupSearchError> {
    let joined_query = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(items) => items.join(" "),
    };
    let depth = text_option(optional_params, "depth")?.unwrap_or_else(|| DEFAULT_DEPTH.to_owned());
    let output_type = text_option(optional_params, "outputType")?
        .unwrap_or_else(|| DEFAULT_OUTPUT_TYPE.to_owned());
    let max_results = max_results_option(optional_params)?;
    let include_domains = domains_option(optional_params)?;
    let extra = passthrough(
        optional_params,
        max_results.as_ref(),
        include_domains.as_deref(),
    );
    Ok(LinkupSearchRequest {
        query: joined_query,
        depth,
        output_type,
        max_results,
        include_domains,
        extra,
    })
}

fn text_option(
    optional_params: &Object,
    field: &'static str,
) -> Result<Option<String>, LinkupSearchError> {
    let Some(value) = optional_params.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .as_str()
        .map_or(Err(LinkupSearchError::OptionType { field }), |text| {
            Ok(Some(text.to_owned()))
        })
}

fn max_results_option(optional_params: &Object) -> Result<Option<i64>, LinkupSearchError> {
    let field = "max_results";
    let Some(value) = optional_params.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value.as_i64().map_or(
        Err(LinkupSearchError::OptionType { field }),
        |max_results| Ok(Some(max_results)),
    )
}

fn domains_option(optional_params: &Object) -> Result<Option<Vec<String>>, LinkupSearchError> {
    let field = "search_domain_filter";
    let Some(value) = optional_params.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Err(LinkupSearchError::OptionType { field });
    };
    let mut domains = Vec::with_capacity(items.len());
    for item in items {
        let Some(domain) = item.as_str() else {
            return Err(LinkupSearchError::OptionType { field });
        };
        domains.push(domain.to_owned());
    }
    Ok(Some(domains))
}

fn passthrough(
    optional_params: &Object,
    max_results: Option<&i64>,
    include_domains: Option<&[String]>,
) -> Object {
    let mut mapped = BTreeSet::from(["q", "depth", "outputType"]);
    if max_results.is_some() {
        mapped.insert("maxResults");
    }
    if include_domains.is_some() {
        mapped.insert("includeDomains");
    }
    let mut extra = Object::new();
    for (name, value) in optional_params {
        if PERPLEXITY_PARAMS.contains(&name.as_str()) || mapped.contains(name.as_str()) {
            continue;
        }
        extra.insert(name.clone(), value.clone());
    }
    extra
}

pub fn decode_linkup_reply(bytes: &[u8]) -> Result<SearchResponse, LinkupSearchError> {
    let reply: LinkupReply =
        serde_json::from_slice(bytes).map_err(|source| LinkupSearchError::ReplyJson { source })?;
    Ok(SearchResponse {
        results: reply.results.into_iter().filter_map(project_item).collect(),
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

#[derive(Debug, Deserialize)]
struct LinkupReply {
    #[serde(default)]
    results: Vec<LinkupItem>,
}

#[derive(Debug, Deserialize)]
struct LinkupItem {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
}

fn project_item(item: LinkupItem) -> Option<SearchResult> {
    let title = match item.kind.as_deref().unwrap_or("text") {
        "text" => item.name.unwrap_or_default(),
        "image" => item.name.unwrap_or_else(|| item.url.clone()),
        _ => return None,
    };
    Some(SearchResult {
        title,
        url: item.url,
        snippet: item.content,
        date: None,
        last_updated: None,
        extra: Object::new(),
    })
}

mod tests;
