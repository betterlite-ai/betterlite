use std::borrow::Cow;
use std::net::{IpAddr, Ipv4Addr};

use betterlite_http::user_url::address::is_blocked_ip;
use betterlite_http::user_url::error::SsrfError;
use betterlite_http::user_url::split::{parse_ipv6, urlparse};
use serde_json::Value;

use super::HostedVllmVideoError;

// _serialize_form_value: a string is itself and every other value its JSON text
#[must_use]
pub fn serialize_form_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

// _maybe_json: a string that starts with { or [ is a JSON reference, and a broken one is an error rather than a literal
fn maybe_json(value: &Value) -> Result<Cow<'_, Value>, HostedVllmVideoError> {
    let Some(text) = value.as_str() else {
        return Ok(Cow::Borrowed(value));
    };
    let stripped = text.trim();
    if stripped.is_empty() || !matches!(stripped.as_bytes().first(), Some(b'{' | b'[')) {
        return Ok(Cow::Borrowed(value));
    }
    serde_json::from_str(stripped)
        .map(Cow::Owned)
        .map_err(|source| HostedVllmVideoError::MediaJson { source })
}

// _form_value: the reference fields are checked for an unsafe media URL before they are written
pub fn form_value(key: &str, value: &Value) -> Result<String, HostedVllmVideoError> {
    reject_unsafe_media_urls(key, value)?;
    Ok(serialize_form_value(value))
}

// _reject_unsafe_media_urls: only the three reference fields carry media URLs, and a bare string URL is checked too
fn reject_unsafe_media_urls(field_name: &str, value: &Value) -> Result<(), HostedVllmVideoError> {
    let url_key = match field_name {
        "image_reference" => "image_url",
        "video_reference" => "video_url",
        "audio_reference" => "audio_url",
        _ => return Ok(()),
    };
    let parsed = maybe_json(value)?;
    match parsed.as_ref() {
        Value::Array(items) => {
            for item in items {
                reject_unsafe_urls_in_item(url_key, item)?;
            }
            Ok(())
        }
        Value::Object(_) => reject_unsafe_urls_in_item(url_key, &parsed),
        Value::String(url) => reject_unsafe_media_url(url),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

// _reject_unsafe_urls_in_item: a reference item that is not an object carries no URL to check
fn reject_unsafe_urls_in_item(url_key: &str, item: &Value) -> Result<(), HostedVllmVideoError> {
    let Value::Object(fields) = item else {
        return Ok(());
    };
    match fields.get(url_key) {
        Some(Value::String(url)) => reject_unsafe_media_url(url),
        _ => Ok(()),
    }
}

// _reject_unsafe_media_url: an empty or data scheme is kept, http(s) needs a hostname and a public destination, every other scheme is refused
fn reject_unsafe_media_url(url: &str) -> Result<(), HostedVllmVideoError> {
    let parsed = urlparse(url, "").map_err(|source| HostedVllmVideoError::UrlParse { source })?;
    if parsed.scheme.is_empty() || parsed.scheme == "data" {
        return Ok(());
    }
    if parsed.scheme != "http" && parsed.scheme != "https" {
        return Err(HostedVllmVideoError::UnsafeUrl(
            SsrfError::SchemeNotAllowed {
                scheme: parsed.scheme,
            },
        ));
    }
    let Some(hostname) = parsed.hostname() else {
        return Err(HostedVllmVideoError::UnsafeUrl(SsrfError::NoHostname));
    };
    parsed
        .port()
        .map_err(|source| HostedVllmVideoError::UrlParse { source })?;
    // a transform resolves no DNS, so it refuses the destinations a name cannot hide: an address literal and localhost
    let literal = literal_address(&hostname);
    let local = hostname == "localhost" || hostname.ends_with(".localhost");
    if literal.is_some_and(is_blocked_ip) || (literal.is_none() && local) {
        let address = literal.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        return Err(HostedVllmVideoError::UnsafeUrl(SsrfError::BlockedAddress {
            address,
        }));
    }
    Ok(())
}

fn literal_address(hostname: &str) -> Option<IpAddr> {
    hostname
        .parse::<IpAddr>()
        .ok()
        .or_else(|| parse_ipv6(hostname).map(IpAddr::V6))
}
