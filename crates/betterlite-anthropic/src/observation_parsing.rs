use std::str;

use betterlite_logging::cache_observer::{
    ObservationParser, ObservedCache, ObservedPrefix, WireRequest,
};
use betterlite_models::decisions::Json;
use http::HeaderMap;
use serde::Deserialize;
use url::Url;

use crate::cache_plan::{PromptPrefix, parse_prompt};
use crate::cache_prediction_targets::headers::supported_prediction_headers;
use crate::cache_prediction_targets::scope::cache_scope;
use crate::cache_prediction_targets::target::supported_provider_key;
use crate::credentials::ANTHROPIC_VERSION;

mod tests;

const NATIVE_HOST: &str = "api.anthropic.com";
const NATIVE_PATH: &str = "/v1/messages";
const NATIVE_PORT: u16 = 443;
const API_KEY_HEADER: &str = "x-api-key";
const VERSION_HEADER: &str = "anthropic-version";
const FIVE_MINUTE_TTL: u32 = 300;
const ONE_HOUR_TTL: u32 = 3_600;
const NATIVE_HEADERS: [&str; 9] = [
    "host",
    "accept",
    "accept-encoding",
    "connection",
    "user-agent",
    "content-length",
    "content-type",
    "x-api-key",
    "anthropic-version",
];

#[derive(Deserialize)]
struct RequestedModel {
    model: String,
}

// LiteLLM's _Response and the usage records nested inside it, all decoded strictly
#[derive(Deserialize)]
struct ObservedResponse {
    model: String,
    usage: ObservedUsage,
    choices: Vec<ObservedChoice>,
}

#[derive(Deserialize)]
struct ObservedUsage {
    prompt_tokens: i64,
    prompt_tokens_details: ObservedCacheUsage,
}

#[derive(Deserialize)]
struct ObservedCacheUsage {
    #[serde(default)]
    cached_tokens: i64,
    #[serde(default)]
    cache_creation_tokens: i64,
    #[serde(default)]
    cache_creation_token_details: Option<ObservedTtlUsage>,
}

#[derive(Deserialize)]
struct ObservedTtlUsage {
    #[serde(default)]
    ephemeral_5m_input_tokens: i64,
    #[serde(default)]
    ephemeral_1h_input_tokens: i64,
}

#[derive(Deserialize)]
struct ObservedChoice {
    finish_reason: String,
}

#[derive(Debug)]
pub struct ObservedCachePrefix {
    pub prefix: PromptPrefix,
    pub scope: String,
    pub cached_tokens: i64,
    pub cache_creation_tokens: i64,
}

#[must_use]
pub fn parse_observed_cache(
    wire: &WireRequest<'_>,
    response: &Json,
    caller_key_hash: &str,
    deployment_id: &str,
) -> Option<ObservedCachePrefix> {
    let observed: ObservedResponse =
        serde_json::from_value(serde_json::to_value(response).ok()?).ok()?;
    if !native_recipient(wire.url) || !native_headers(wire.headers) {
        return None;
    }
    let provider_key = native_credentials(wire.headers)?;
    let body = str::from_utf8(wire.body).ok()?;
    let model = body_model(body)?;
    if model != observed.model {
        return None;
    }
    let prefix = parse_prompt(body)?;
    let usage = &observed.usage;
    if usage.prompt_tokens < 0 {
        return None;
    }
    let complete = observed
        .choices
        .iter()
        .all(|choice| !choice.finish_reason.is_empty());
    if observed.choices.is_empty() || !complete {
        return None;
    }
    let cache_tokens = cache_counts(
        &usage.prompt_tokens_details,
        usage.prompt_tokens,
        prefix.ttl_seconds,
    )?;
    Some(ObservedCachePrefix {
        prefix,
        scope: cache_scope(caller_key_hash, deployment_id, provider_key, &model),
        cached_tokens: cache_tokens,
        cache_creation_tokens: usage.prompt_tokens_details.cache_creation_tokens,
    })
}

fn native_recipient(url: &Url) -> bool {
    let secure = url.scheme() == "https";
    let native_host = url.host_str() == Some(NATIVE_HOST);
    let native_path = url.path() == NATIVE_PATH;
    let without_query = url.query().is_none_or(str::is_empty);
    let default_port = url.port().is_none_or(|port| port == NATIVE_PORT);
    secure && native_host && native_path && without_query && default_port
}

fn native_headers(headers: &HeaderMap) -> bool {
    headers
        .keys()
        .all(|name| NATIVE_HEADERS.contains(&name.as_str()))
        && supported_prediction_headers(headers)
}

// httpx collapses a repeated header to its last value before LiteLLM validates the dict
fn native_credentials(headers: &HeaderMap) -> Option<&str> {
    let version = headers.get_all(VERSION_HEADER).iter().last()?;
    if version.as_bytes() != ANTHROPIC_VERSION.as_bytes() {
        return None;
    }
    let api_key = headers.get_all(API_KEY_HEADER).iter().last()?;
    let key = api_key.to_str().ok()?;
    if !supported_provider_key(key) {
        return None;
    }
    Some(key)
}

fn body_model(body: &str) -> Option<String> {
    match serde_json::from_str::<RequestedModel>(body) {
        Ok(requested) => Some(requested.model),
        Err(_) => None,
    }
}

fn cache_counts(usage: &ObservedCacheUsage, prompt_tokens: i64, ttl_seconds: u32) -> Option<i64> {
    if usage.cached_tokens < 0 || usage.cache_creation_tokens < 0 {
        return None;
    }
    let cache_tokens = usage
        .cached_tokens
        .checked_add(usage.cache_creation_tokens)?;
    if cache_tokens == 0 || cache_tokens > prompt_tokens {
        return None;
    }
    let split = usage.cache_creation_token_details.as_ref();
    if usage.cache_creation_tokens != 0 && split.is_none() {
        return None;
    }
    if let Some(details) = split
        && !split_matches(details, usage.cache_creation_tokens, ttl_seconds)
    {
        return None;
    }
    Some(cache_tokens)
}

fn split_matches(details: &ObservedTtlUsage, creation: i64, ttl_seconds: u32) -> bool {
    if details.ephemeral_5m_input_tokens < 0 || details.ephemeral_1h_input_tokens < 0 {
        return false;
    }
    let total = details
        .ephemeral_5m_input_tokens
        .checked_add(details.ephemeral_1h_input_tokens);
    let one_hour_split = details.ephemeral_1h_input_tokens > 0;
    let five_minute_split = details.ephemeral_5m_input_tokens > 0;
    let ttl_allows = (ttl_seconds != FIVE_MINUTE_TTL || !one_hour_split)
        && (ttl_seconds != ONE_HOUR_TTL || !five_minute_split);
    total == Some(creation) && ttl_allows
}

pub struct ObservedCacheParser;

impl ObservationParser for ObservedCacheParser {
    fn parse(
        &self,
        wire: &WireRequest<'_>,
        response: &Json,
        caller_key_hash: &str,
        deployment_id: &str,
    ) -> Option<ObservedCache> {
        let observed = parse_observed_cache(wire, response, caller_key_hash, deployment_id)?;
        Some(ObservedCache {
            prefix: ObservedPrefix {
                fingerprint: observed.prefix.fingerprint,
                fingerprints: observed.prefix.fingerprints,
            },
            ttl_seconds: observed.prefix.ttl_seconds,
            scope: observed.scope,
            cached_tokens: observed.cached_tokens,
            cache_creation_tokens: observed.cache_creation_tokens,
        })
    }
}
