use http::HeaderMap;
use http::header::{AUTHORIZATION, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::context::GatewayTarget;
use crate::payloads::python_json::python_json_text;
use crate::types::{Harness, JsonObject};

const HOP_BY_HOP_HEADERS: [&str; 11] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];
const REQUEST_ONLY_DROPPED: [&str; 3] = ["authorization", "x-api-key", "accept-encoding"];
const RESPONSE_ONLY_DROPPED: [&str; 1] = ["content-encoding"];
const BEARER_SCHEME: &str = "bearer ";

#[derive(Debug, Error)]
pub enum HeaderError {
    #[error("the {name} header value is not valid HTTP")]
    Value {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the spend log metadata could not be encoded")]
    Metadata {
        #[source]
        source: serde_json::Error,
    },
}

#[must_use]
pub fn extract_token(headers: &HeaderMap) -> Option<SecretString> {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let bearer = authorization
        .split_at_checked(BEARER_SCHEME.len())
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case(BEARER_SCHEME));
    if let Some((_, token)) = bearer {
        return Some(SecretString::from(token.trim().to_owned()));
    }
    headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(|key| SecretString::from(key.to_owned()))
}

pub fn gateway_headers(
    incoming: &HeaderMap,
    gateway: &GatewayTarget,
    harness: Harness,
    metadata: &JsonObject,
) -> Result<HeaderMap, HeaderError> {
    let mut headers = HeaderMap::new();
    for (name, value) in incoming {
        let dropped = HOP_BY_HOP_HEADERS.contains(&name.as_str())
            || REQUEST_ONLY_DROPPED.contains(&name.as_str())
            || name.as_str().starts_with("x-litellm-");
        if !dropped {
            headers.append(name.clone(), value.clone());
        }
    }
    let bearer = format!("Bearer {}", gateway.api_key.expose_secret());
    let mut authorization =
        HeaderValue::from_str(&bearer).map_err(|source| HeaderError::Value {
            name: "authorization",
            source,
        })?;
    authorization.set_sensitive(true);
    headers.insert(AUTHORIZATION, authorization);
    let tag_text = format!("harness,{}", harness_tag(harness));
    let tags = HeaderValue::from_str(&tag_text).map_err(|source| HeaderError::Value {
        name: "x-litellm-tags",
        source,
    })?;
    headers.insert("x-litellm-tags", tags);
    if !metadata.is_empty() {
        let encoded =
            python_json_text(metadata).map_err(|source| HeaderError::Metadata { source })?;
        let value = HeaderValue::from_str(&encoded).map_err(|source| HeaderError::Value {
            name: "x-litellm-spend-logs-metadata",
            source,
        })?;
        headers.insert("x-litellm-spend-logs-metadata", value);
    }
    Ok(headers)
}

#[must_use]
pub fn response_headers(upstream: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in upstream {
        let dropped = HOP_BY_HOP_HEADERS.contains(&name.as_str())
            || RESPONSE_ONLY_DROPPED.contains(&name.as_str());
        if !dropped {
            headers.append(name.clone(), value.clone());
        }
    }
    headers
}

const fn harness_tag(harness: Harness) -> &'static str {
    match harness {
        Harness::ClaudeCode => "claude_code",
        Harness::Codex => "codex",
        Harness::OpenCode => "opencode",
        Harness::DeepAgents => "deepagents",
    }
}
