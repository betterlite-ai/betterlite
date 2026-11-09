use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::{CredentialError, ServerCredential};
use betterlite_openai::search::provider::{
    OutboundSearch, PERPLEXITY_PARAMS, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};
use thiserror::Error;

pub const SERPER_API_BASE: &str = "https://google.serper.dev";
const API_KEY_VARIABLE: &str = "SERPER_API_KEY";
const API_BASE_VARIABLE: &str = "SERPER_API_BASE";
const API_KEY_HEADER: &str = "X-API-KEY";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";
const SEARCH_SUFFIX: &str = "/search";
const REPLY_MAX_BYTES: usize = 0x0100_0000;

const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[API_KEY_VARIABLE],
    base_var: Some(API_BASE_VARIABLE),
    default_base: Some(SERPER_API_BASE),
};

#[derive(Debug, Error)]
pub enum SerperSearchError {
    #[error("SERPER_API_KEY is not set. Set `SERPER_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the Serper search option `{field}` is not a string")]
    OptionType { field: &'static str },
    #[error("the Serper search option `{field}` is not a list of strings")]
    OptionList { field: &'static str },
    #[error("a Serper search request option is not JSON")]
    RequestJson(#[source] serde_json::Error),
    #[error("the Serper search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the Serper search reply does not match the Serper schema")]
    ReplyDecode(#[source] serde_json::Error),
    #[error("the Serper search reply field `organic` is null")]
    OrganicShape,
    #[error("the Serper search reply result field `{field}` is null")]
    ResultField { field: &'static str },
    #[error("the Serper search reply result field `{field}` has the wrong type")]
    ReplyField {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone)]
pub struct SerperSearchSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl SerperSearchSettings {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(API_KEY_VARIABLE).map(SecretString::from),
            api_base: present(API_BASE_VARIABLE).unwrap_or_else(|| SERPER_API_BASE.to_owned()),
        }
    }
}

impl EnvironmentLookup for SerperSearchSettings {
    fn lookup(&self, name: &str) -> Option<String> {
        if name == API_KEY_VARIABLE {
            return self
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned());
        }
        (name == API_BASE_VARIABLE).then(|| self.api_base.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerperSearchRequest {
    #[serde(rename = "q")]
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num: Option<Box<RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gl: Option<String>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Debug, Clone)]
pub struct SerperSearchConfig {
    pub settings: SerperSearchSettings,
}

impl SerperSearchConfig {
    pub fn validated_headers(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, SerperSearchError> {
        let Some(key) = CREDENTIAL.resolve(api_key.cloned(), api_base, &self.settings)? else {
            return Err(SerperSearchError::MissingApiKey);
        };
        headers.insert(API_KEY_HEADER.to_owned(), key.expose_secret().to_owned());
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        Ok(headers)
    }
}

impl SearchProvider for SerperSearchConfig {
    type Body = SerperSearchRequest;
    type Error = SerperSearchError;

    fn ui_friendly_name() -> &'static str {
        "Serper"
    }

    // B3270's hook returns headers only; the same check fails in sign_request before any request leaves
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        self.validated_headers(headers.clone(), api_key, api_base)
            .unwrap_or(headers)
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&Self::Body>,
    ) -> Result<String, Self::Error> {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(&self.settings.api_base)
            .trim_end_matches('/');
        if base.ends_with(SEARCH_SUFFIX) {
            return Ok(base.to_owned());
        }
        Ok(format!("{base}{SEARCH_SUFFIX}"))
    }

    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        let validated = self.validated_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((validated, None))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<Self::Body, Self::Error> {
        let joined = match query {
            SearchQuery::Single(text) => text.clone(),
            SearchQuery::Multiple(queries) => queries.join(" "),
        };
        let mut body = SerperSearchRequest {
            query: joined,
            num: raw_field(optional_params, "max_results")?,
            gl: serper_country(optional_params)?,
            extra: Object::new(),
        };
        if let Some(clauses) = serper_domains(optional_params)? {
            body.query = format!("({}) ({})", body.query, clauses.join(" OR "));
        }
        let mut extra = Object::new();
        for (name, value) in optional_params {
            if PERPLEXITY_PARAMS.contains(&name.as_str()) {
                continue;
            }
            if serper_already_in_body(name, optional_params) {
                continue;
            }
            extra.insert(name.clone(), value.clone());
        }
        body.extra = extra;
        Ok(body)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Self::Error> {
        decode_serper_search(response.text.as_bytes())
    }
}

fn raw_field(
    optional_params: &Object,
    name: &str,
) -> Result<Option<Box<RawValue>>, SerperSearchError> {
    optional_params
        .get(name)
        .map(|value| to_raw_value(value).map_err(SerperSearchError::RequestJson))
        .transpose()
}

fn serper_country(optional_params: &Object) -> Result<Option<String>, SerperSearchError> {
    let Some(value) = optional_params.get("country") else {
        return Ok(None);
    };
    value.as_str().map_or_else(
        || Err(SerperSearchError::OptionType { field: "country" }),
        |country| Ok(Some(country.to_lowercase())),
    )
}

fn serper_domains(optional_params: &Object) -> Result<Option<Vec<String>>, SerperSearchError> {
    let Some(value) = optional_params.get("search_domain_filter") else {
        return Ok(None);
    };
    let Some(domains) = value.as_array() else {
        return Ok(None);
    };
    if domains.is_empty() {
        return Ok(None);
    }
    let mut clauses = Vec::with_capacity(domains.len());
    for member in domains {
        let Some(domain) = member.as_str() else {
            return Err(SerperSearchError::OptionList {
                field: "search_domain_filter",
            });
        };
        clauses.push(format!("site:{domain}"));
    }
    Ok(Some(clauses))
}

// the unified name only fills its Serper field when it is present, and the copy check runs after that
fn serper_already_in_body(name: &str, optional_params: &Object) -> bool {
    let source = match name {
        "q" => return true,
        "num" => "max_results",
        "gl" => "country",
        _ => return false,
    };
    optional_params.contains_key(source)
}

#[derive(Debug, Deserialize)]
struct SerperReply {
    #[serde(default)]
    organic: Option<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
struct SerperReplyResult {
    #[serde(default)]
    title: Option<Box<RawValue>>,
    #[serde(default)]
    link: Option<Box<RawValue>>,
    #[serde(default)]
    snippet: Option<Box<RawValue>>,
    #[serde(default)]
    date: Option<Box<RawValue>>,
}

pub fn decode_serper_search(bytes: &[u8]) -> Result<SearchResponse, SerperSearchError> {
    if bytes.len() > REPLY_MAX_BYTES {
        return Err(SerperSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    let reply: SerperReply =
        serde_json::from_slice(bytes).map_err(SerperSearchError::ReplyDecode)?;
    let results = match reply.organic {
        None => Vec::new(),
        Some(raw) => {
            if serde_json::from_str::<()>(raw.get()).is_ok() {
                return Err(SerperSearchError::OrganicShape);
            }
            let items: Vec<SerperReplyResult> =
                serde_json::from_str(raw.get()).map_err(SerperSearchError::ReplyDecode)?;
            items
                .into_iter()
                .map(serper_result)
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn serper_result(result: SerperReplyResult) -> Result<SearchResult, SerperSearchError> {
    Ok(SearchResult {
        title: text_member(result.title, "title")?,
        url: text_member(result.link, "link")?,
        snippet: text_member(result.snippet, "snippet")?,
        date: date_member(result.date, "date")?,
        last_updated: None,
        extra: Object::new(),
    })
}

fn text_member(
    value: Option<Box<RawValue>>,
    name: &'static str,
) -> Result<String, SerperSearchError> {
    let Some(raw) = value else {
        return Ok(String::new());
    };
    if serde_json::from_str::<()>(raw.get()).is_ok() {
        return Err(SerperSearchError::ResultField { field: name });
    }
    serde_json::from_str(raw.get()).map_err(|source| SerperSearchError::ReplyField {
        field: name,
        source,
    })
}

fn date_member(
    value: Option<Box<RawValue>>,
    name: &'static str,
) -> Result<Option<String>, SerperSearchError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    if serde_json::from_str::<()>(raw.get()).is_ok() {
        return Ok(None);
    }
    serde_json::from_str(raw.get())
        .map(Some)
        .map_err(|source| SerperSearchError::ReplyField {
            field: name,
            source,
        })
}

mod tests;
