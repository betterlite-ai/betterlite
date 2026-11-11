use betterlite_error::envelope::is_unsafe_header_name;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::decisions::Json;

pub type AdditionalHeaders = IndexMap<String, Json>;

// LiteLLM's OPENAI_RESPONSE_HEADERS: repeated bare next to the llm_provider- copy
pub const OPENAI_RESPONSE_HEADERS: [&str; 6] = [
    "x-ratelimit-remaining-requests",
    "x-ratelimit-remaining-tokens",
    "x-ratelimit-limit-requests",
    "x-ratelimit-limit-tokens",
    "x-ratelimit-reset-requests",
    "x-ratelimit-reset-tokens",
];

const LLM_PROVIDER_PREFIX: &str = "llm_provider-";
const INTERNAL_HEADER_PREFIX: &str = "x-litellm-";

// proxy-written headers: request parsing never builds this map
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedHeaders {
    pub entries: AdditionalHeaders,
}

#[derive(Debug, Clone, Copy)]
pub enum HeaderSource<'headers> {
    Upstream(&'headers IndexMap<String, String>),
    Internal(&'headers TrustedHeaders),
}

#[derive(Debug, Default)]
struct HeaderProjection {
    repeated: AdditionalHeaders,
    passed_through: AdditionalHeaders,
    prefixed: AdditionalHeaders,
}

impl HeaderProjection {
    fn place(&mut self, name: &str, value: Json, preserve_internal: bool) {
        if OPENAI_RESPONSE_HEADERS.contains(&name) {
            self.repeated.insert(name.to_owned(), value.clone());
        }
        let keeps_name = name.starts_with(LLM_PROVIDER_PREFIX)
            || (preserve_internal && name.starts_with(INTERNAL_HEADER_PREFIX));
        if keeps_name {
            self.passed_through.insert(name.to_owned(), value);
        } else {
            self.prefixed
                .insert(format!("{LLM_PROVIDER_PREFIX}{name}"), value);
        }
    }

    fn merged(self) -> AdditionalHeaders {
        let mut headers = self.repeated;
        headers.extend(self.passed_through);
        headers.extend(self.prefixed);
        headers.retain(|name, _value| !is_unsafe_header_name(name));
        headers
    }
}

#[must_use]
pub fn process_response_headers(source: HeaderSource<'_>) -> AdditionalHeaders {
    let mut projection = HeaderProjection::default();
    match source {
        HeaderSource::Upstream(headers) => {
            for (name, value) in headers {
                projection.place(name, Json::Text(value.clone()), false);
            }
        }
        HeaderSource::Internal(trusted) => {
            for (name, value) in &trusted.entries {
                projection.place(name, value.clone(), true);
            }
        }
    }
    projection.merged()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderHeaderProjection {
    pub headers: IndexMap<String, String>,
    pub additional_headers: AdditionalHeaders,
}

#[must_use]
pub fn set_provider_response_headers(
    existing_additional_headers: &AdditionalHeaders,
    provider_headers: &IndexMap<String, String>,
) -> ProviderHeaderProjection {
    let mut additional_headers = process_response_headers(HeaderSource::Upstream(provider_headers));
    additional_headers.extend(existing_additional_headers.clone());
    ProviderHeaderProjection {
        headers: provider_headers.clone(),
        additional_headers,
    }
}

// the stored hidden headers are strings only; any other value fails the mapping validation
#[must_use]
pub fn provider_response_headers(hidden: Option<&Json>) -> Option<IndexMap<String, String>> {
    let Json::Object(fields) = hidden? else {
        return None;
    };
    let Json::Object(headers) = fields.get("headers")? else {
        return None;
    };
    let validated: Option<IndexMap<String, String>> = headers
        .iter()
        .map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect();
    validated
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ProxyServerRequest {
    pub headers: Option<IndexMap<String, String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RequestParams {
    pub proxy_server_request: Option<ProxyServerRequest>,
}

#[must_use]
pub fn proxy_server_request_headers(params: Option<&RequestParams>) -> IndexMap<String, String> {
    params
        .and_then(|given| given.proxy_server_request.as_ref())
        .and_then(|request| request.headers.clone())
        .unwrap_or_default()
}
