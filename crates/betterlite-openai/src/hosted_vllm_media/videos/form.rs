use std::borrow::Cow;
use std::net::{IpAddr, Ipv4Addr};

use betterlite_http::user_url::address::is_blocked_ip;
use betterlite_http::user_url::error::SsrfError;
use betterlite_http::user_url::split::{parse_ipv6, urlparse};
use serde_json::Value;

use super::HostedVllmVideoError;

// _serialize_form_value: a string is itself and every other value its JSON text
#[must_use]
fn serialize_form_value(value: &Value) -> String {
    let Value::String(text) = value else {
        return value.to_string();
    };
    text.clone()
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
pub(super) fn form_value(
    key: &str,
    value: &Value,
    allowed_hosts: &[String],
) -> Result<String, HostedVllmVideoError> {
    reject_unsafe_media_urls(key, value, allowed_hosts)?;
    Ok(serialize_form_value(value))
}

// _reject_unsafe_media_urls: only the three reference fields carry media URLs, and a bare string URL is checked too
fn reject_unsafe_media_urls(
    field_name: &str,
    value: &Value,
    allowed_hosts: &[String],
) -> Result<(), HostedVllmVideoError> {
    let url_key = match field_name {
        "image_reference" => "image_url",
        "video_reference" => "video_url",
        "audio_reference" => "audio_url",
        _ => return Ok(()),
    };
    let parsed = maybe_json(value)?;
    match parsed.as_ref() {
        Value::Array(items) => items
            .iter()
            .try_for_each(|item| reject_unsafe_urls_in_item(url_key, item, allowed_hosts)),
        Value::Object(_) => reject_unsafe_urls_in_item(url_key, &parsed, allowed_hosts),
        Value::String(url) => reject_unsafe_media_url(url, allowed_hosts),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

// _reject_unsafe_urls_in_item: a reference item that is not an object carries no URL to check
fn reject_unsafe_urls_in_item(
    url_key: &str,
    item: &Value,
    allowed_hosts: &[String],
) -> Result<(), HostedVllmVideoError> {
    let Value::Object(fields) = item else {
        return Ok(());
    };
    fields
        .get(url_key)
        .and_then(Value::as_str)
        .map_or(Ok(()), |url| reject_unsafe_media_url(url, allowed_hosts))
}

// _is_host_allowlisted: an entry is a bare host or host:port, lowercased with any trailing dot ignored, and an IPv6 host stays bracketed
fn host_is_allowlisted(hostname: &str, port: u16, allowed_hosts: &[String]) -> bool {
    let lowered = hostname.to_lowercase();
    let host = lowered.trim_end_matches('.');
    let shown = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let with_port = format!("{shown}:{port}");
    allowed_hosts.iter().any(|entry| {
        let entry_lowered = entry.to_lowercase();
        let entry_host = entry_lowered.trim_end_matches('.');
        entry_host == shown || entry_host == with_port
    })
}

// _reject_unsafe_media_url: an empty or data scheme is kept, http(s) needs a hostname and a public destination, every other scheme is refused
fn reject_unsafe_media_url(
    url: &str,
    allowed_hosts: &[String],
) -> Result<(), HostedVllmVideoError> {
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
    let port = parsed
        .port()
        .map_err(|source| HostedVllmVideoError::UrlParse { source })?;
    // a transform resolves no DNS, so it refuses the destinations a name cannot hide: an address literal and localhost, unless the admin allowlisted the host
    let literal = hostname
        .parse::<IpAddr>()
        .ok()
        .or_else(|| parse_ipv6(&hostname).map(IpAddr::V6));
    let local = hostname == "localhost" || hostname.ends_with(".localhost");
    let default_port = if parsed.scheme == "https" { 443 } else { 80 };
    if (literal.is_some_and(is_blocked_ip) || (literal.is_none() && local))
        && !host_is_allowlisted(&hostname, port.unwrap_or(default_port), allowed_hosts)
    {
        let address = literal.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        return Err(HostedVllmVideoError::UnsafeUrl(SsrfError::BlockedAddress {
            address,
        }));
    }
    Ok(())
}
