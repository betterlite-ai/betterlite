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

pub const EXA_AI_API_BASE: &str = "https://api.exa.ai";
const API_KEY_VARIABLE: &str = "EXA_API_KEY";
const API_BASE_VARIABLE: &str = "EXA_API_BASE";
const API_KEY_HEADER: &str = "x-api-key";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";
const SEARCH_SUFFIX: &str = "/search";
const REPLY_MAX_BYTES: usize = 0x0100_0000;

const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[API_KEY_VARIABLE],
    base_var: Some(API_BASE_VARIABLE),
    default_base: Some(EXA_AI_API_BASE),
};

#[derive(Debug, Error)]
pub enum ExaAiSearchError {
    #[error("EXA_API_KEY is not set. Set `EXA_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("an Exa AI search request option is not JSON")]
    RequestJson(#[source] serde_json::Error),
    #[error("the Exa AI search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the Exa AI search reply does not match the Exa AI schema")]
    ReplyDecode(#[source] serde_json::Error),
    #[error("the Exa AI search reply field `results` is null")]
    ResultsShape,
    #[error("the Exa AI search reply result field `{field}` is null")]
    ResultField { field: &'static str },
    #[error("the Exa AI search reply result field `{field}` has the wrong type")]
    ReplyField {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone)]
pub struct ExaAiSearchSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl ExaAiSearchSettings {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(API_KEY_VARIABLE).map(SecretString::from),
            api_base: present(API_BASE_VARIABLE).unwrap_or_else(|| EXA_AI_API_BASE.to_owned()),
        }
    }
}

impl EnvironmentLookup for ExaAiSearchSettings {
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
pub struct ExaAiSearchRequest {
    pub query: String,
    #[serde(rename = "numResults", skip_serializing_if = "Option::is_none")]
    pub num_results: Option<Box<RawValue>>,
    #[serde(rename = "includeDomains", skip_serializing_if = "Option::is_none")]
    pub include_domains: Option<Box<RawValue>>,
    #[serde(rename = "userLocation", skip_serializing_if = "Option::is_none")]
    pub user_location: Option<Box<RawValue>>,
    pub contents: Box<RawValue>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Debug, Serialize)]
struct DefaultContents {
    text: bool,
}

#[derive(Debug, Clone)]
pub struct ExaAiSearchConfig {
    pub settings: ExaAiSearchSettings,
}

impl ExaAiSearchConfig {
    pub fn validated_headers(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, ExaAiSearchError> {
        let Some(key) = CREDENTIAL.resolve(api_key.cloned(), api_base, &self.settings)? else {
            return Err(ExaAiSearchError::MissingApiKey);
        };
        headers.insert(API_KEY_HEADER.to_owned(), key.expose_secret().to_owned());
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        Ok(headers)
    }
}

impl SearchProvider for ExaAiSearchConfig {
    type Body = ExaAiSearchRequest;
    type Error = ExaAiSearchError;

    fn ui_friendly_name() -> &'static str {
        "Exa AI"
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
            .unwrap_or(&self.settings.api_base);
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
        let mut extra = Object::new();
        for (name, value) in optional_params {
            if PERPLEXITY_PARAMS.contains(&name.as_str()) || name == "contents" {
                continue;
            }
            if is_already_in_body(name, optional_params) {
                continue;
            }
            extra.insert(name.clone(), value.clone());
        }
        let contents = match raw_field(optional_params, "contents")? {
            Some(contents) => contents,
            None => to_raw_value(&DefaultContents { text: true })
                .map_err(ExaAiSearchError::RequestJson)?,
        };
        Ok(ExaAiSearchRequest {
            query: joined,
            num_results: raw_field(optional_params, "max_results")?,
            include_domains: raw_field(optional_params, "search_domain_filter")?,
            user_location: raw_field(optional_params, "country")?,
            contents,
            extra,
        })
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Self::Error> {
        decode_exa_ai_search(response.text.as_bytes())
    }
}

fn raw_field(
    optional_params: &Object,
    name: &str,
) -> Result<Option<Box<RawValue>>, ExaAiSearchError> {
    optional_params
        .get(name)
        .map(|value| to_raw_value(value).map_err(ExaAiSearchError::RequestJson))
        .transpose()
}

// the unified name only fills its Exa field when it is present, and the copy check runs after that
fn is_already_in_body(name: &str, optional_params: &Object) -> bool {
    let source = match name {
        "query" => return true,
        "numResults" => "max_results",
        "includeDomains" => "search_domain_filter",
        "userLocation" => "country",
        _ => return false,
    };
    optional_params.contains_key(source)
}

#[derive(Debug, Deserialize)]
struct ExaReply {
    #[serde(default)]
    results: Option<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
struct ExaReplyResult {
    #[serde(default)]
    title: Option<Box<RawValue>>,
    #[serde(default)]
    url: Option<Box<RawValue>>,
    #[serde(default)]
    text: Option<Box<RawValue>>,
    #[serde(default, rename = "publishedDate")]
    published_date: Option<Box<RawValue>>,
}

pub fn decode_exa_ai_search(bytes: &[u8]) -> Result<SearchResponse, ExaAiSearchError> {
    if bytes.len() > REPLY_MAX_BYTES {
        return Err(ExaAiSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    let reply: ExaReply = serde_json::from_slice(bytes).map_err(ExaAiSearchError::ReplyDecode)?;
    let results = match reply.results {
        None => Vec::new(),
        Some(raw) => {
            if serde_json::from_str::<()>(raw.get()).is_ok() {
                return Err(ExaAiSearchError::ResultsShape);
            }
            let items: Vec<ExaReplyResult> =
                serde_json::from_str(raw.get()).map_err(ExaAiSearchError::ReplyDecode)?;
            items
                .into_iter()
                .map(exa_result)
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

fn exa_result(result: ExaReplyResult) -> Result<SearchResult, ExaAiSearchError> {
    Ok(SearchResult {
        title: text_member(result.title, "title")?,
        url: text_member(result.url, "url")?,
        snippet: text_member(result.text, "text")?,
        date: date_member(result.published_date, "publishedDate")?,
        last_updated: None,
        extra: Object::new(),
    })
}

fn text_member(
    value: Option<Box<RawValue>>,
    name: &'static str,
) -> Result<String, ExaAiSearchError> {
    let Some(raw) = value else {
        return Ok(String::new());
    };
    if serde_json::from_str::<()>(raw.get()).is_ok() {
        return Err(ExaAiSearchError::ResultField { field: name });
    }
    serde_json::from_str(raw.get()).map_err(|source| ExaAiSearchError::ReplyField {
        field: name,
        source,
    })
}

fn date_member(
    value: Option<Box<RawValue>>,
    name: &'static str,
) -> Result<Option<String>, ExaAiSearchError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    if serde_json::from_str::<()>(raw.get()).is_ok() {
        return Ok(None);
    }
    serde_json::from_str(raw.get())
        .map(Some)
        .map_err(|source| ExaAiSearchError::ReplyField {
            field: name,
            source,
        })
}

mod tests;
