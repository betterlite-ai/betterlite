mod tests;

use std::net::Ipv4Addr;

use betterlite_http::user_url::split::{Parsed, parse_ipv6, urlparse};
use http::StatusCode;
use serde::Serialize;
use thiserror::Error;

pub const DEFAULT_NATIVE_REDIRECT_URIS: [&str; 1] =
    ["cursor://anysphere.cursor-mcp/oauth/callback"];

pub const INVALID_REQUEST: &str = "invalid_request";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{description}")]
pub struct RedirectUriRejected {
    pub description: String,
    pub hint: Option<String>,
    pub redirect_uri: Option<String>,
}

impl RedirectUriRejected {
    pub const STATUS: StatusCode = StatusCode::BAD_REQUEST;

    #[must_use]
    pub fn detail(&self) -> RejectionDetail<'_> {
        RejectionDetail {
            error: INVALID_REQUEST,
            error_description: &self.description,
            hint: self.hint.as_deref(),
            redirect_uri: self.redirect_uri.as_deref(),
        }
    }
}

// FastAPI writes the HTTPException detail under a "detail" key; this is that object
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RejectionDetail<'text> {
    pub error: &'static str,
    pub error_description: &'text str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<&'text str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<&'text str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectShape {
    NativeCallback,
    NeedsTrustPolicy,
}

#[must_use]
pub fn native_callbacks(configured: &str) -> Vec<String> {
    let mut callbacks: Vec<String> = DEFAULT_NATIVE_REDIRECT_URIS
        .iter()
        .copied()
        .map(str::to_lowercase)
        .collect();
    for token in configured.trim().split(',') {
        let entry = token.trim().to_lowercase();
        if !entry.is_empty() && !callbacks.contains(&entry) {
            callbacks.push(entry);
        }
    }
    callbacks
}

pub fn parse_redirect_uri(redirect_uri: &str) -> Result<Parsed, RedirectUriRejected> {
    urlparse(redirect_uri, "")
        .ok()
        .ok_or(RedirectUriRejected {
            description: "redirect_uri is not a valid URL.".to_owned(),
            hint: Some(
                "Use a full absolute URL for redirect_uri (e.g. https://your-host/ui/mcp/oauth/callback)."
                    .to_owned(),
            ),
            redirect_uri: None,
        })
}

#[must_use]
pub fn is_loopback_redirect_host(parsed: &Parsed) -> bool {
    let Some(host) = parsed.hostname() else {
        return false;
    };
    if host == "localhost" {
        return true;
    }
    host.parse::<Ipv4Addr>()
        .is_ok_and(|address| address.is_loopback())
        || parse_ipv6(&host).is_some_and(|address| {
            address.is_loopback()
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| mapped.is_loopback())
        })
}

pub fn validate_redirect_uri_shape(
    parsed: &Parsed,
    native_callbacks: &[String],
) -> Result<RedirectShape, RedirectUriRejected> {
    if parsed.scheme != "http" && parsed.scheme != "https" {
        if matches_native_callback(parsed, native_callbacks) {
            return Ok(RedirectShape::NativeCallback);
        }
        let scheme = parsed.scheme.as_str();
        return Err(RedirectUriRejected {
            description: format!(
                "redirect_uri scheme '{scheme}' is not allowed; use http/https or a registered native callback (e.g. cursor://)."
            ),
            hint: Some(
                "Add the full URI to MCP_TRUSTED_NATIVE_REDIRECT_URIS for custom native clients."
                    .to_owned(),
            ),
            redirect_uri: None,
        });
    }
    match web_redirect_uri_rejection(parsed) {
        Some(rejected) => Err(rejected),
        None => Ok(RedirectShape::NeedsTrustPolicy),
    }
}

fn web_redirect_uri_rejection(parsed: &Parsed) -> Option<RedirectUriRejected> {
    if !parsed.fragment.is_empty() {
        return Some(RedirectUriRejected {
            description: "redirect_uri must not contain a URL fragment (#...).".to_owned(),
            hint: None,
            redirect_uri: None,
        });
    }
    if parsed.netloc.is_empty() {
        return Some(RedirectUriRejected {
            description: "redirect_uri must include a host (e.g. https://your-host/path)."
                .to_owned(),
            hint: None,
            redirect_uri: None,
        });
    }
    if parsed.netloc.contains('@') {
        return Some(RedirectUriRejected {
            description: "redirect_uri must not contain userinfo (user:pass@host).".to_owned(),
            hint: None,
            redirect_uri: None,
        });
    }
    if parsed.netloc.contains('\\') {
        return Some(RedirectUriRejected {
            description: "redirect_uri host must not contain backslashes.".to_owned(),
            hint: None,
            redirect_uri: None,
        });
    }
    None
}

fn matches_native_callback(parsed: &Parsed, native_callbacks: &[String]) -> bool {
    if !parsed.fragment.is_empty()
        || !parsed.query.is_empty()
        || parsed.netloc.is_empty()
        || parsed.netloc.contains('@')
        || parsed.netloc.contains('\\')
    {
        return false;
    }
    let normalized = normalize_native_redirect_uri(parsed);
    native_callbacks
        .iter()
        .any(|entry| match entry.strip_suffix('*') {
            Some(prefix) => native_wildcard_prefix_matches(&normalized, prefix),
            None => normalized == *entry,
        })
}

fn normalize_native_redirect_uri(parsed: &Parsed) -> String {
    Parsed {
        scheme: parsed.scheme.to_lowercase(),
        netloc: parsed.netloc.to_lowercase(),
        path: parsed.path.to_lowercase(),
        params: String::new(),
        query: String::new(),
        fragment: String::new(),
    }
    .unparse()
}

fn native_wildcard_prefix_matches(normalized: &str, prefix: &str) -> bool {
    let Some(suffix) = normalized.strip_prefix(prefix) else {
        return false;
    };
    if suffix.is_empty() || prefix.ends_with('/') {
        return true;
    }
    suffix.starts_with('/')
}
