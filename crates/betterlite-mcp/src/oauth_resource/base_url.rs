mod tests;

use tracing::warn;

use super::python_url::{self, UrlParseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestOrigin<'origin> {
    pub base_url: &'origin str,
    pub x_forwarded_proto: Option<&'origin str>,
    pub x_forwarded_host: Option<&'origin str>,
    pub x_forwarded_port: Option<&'origin str>,
    pub from_trusted_proxy: bool,
}

// LiteLLM compares scheme case-sensitively here, so an uppercase forwarded proto keeps its default port.
#[must_use]
pub fn strip_default_port(scheme: &str, netloc: &str) -> String {
    if netloc.is_empty() {
        return String::new();
    }
    let lowered = netloc.to_lowercase();
    let default_port = match scheme {
        "http" => Some(80_i128),
        "https" => Some(443_i128),
        _ => None,
    };
    if lowered.starts_with('[') {
        return strip_ipv6_default_port(&lowered, default_port);
    }
    if let Some((host, port_text)) = lowered.rsplit_once(':')
        && let Some(port) = python_url::netloc::int_value(port_text)
        && default_port == Some(port)
    {
        return host.to_owned();
    }
    lowered
}

#[must_use]
pub fn normalize_proxy_base_url(raw: &str) -> Option<String> {
    let configured = raw.trim();
    if configured.is_empty() {
        return None;
    }
    let usable = python_url::parse(configured).ok().filter(|parsed| {
        matches!(parsed.scheme.as_str(), "http" | "https") && !parsed.netloc.is_empty()
    });
    let Some(parsed) = usable else {
        warn!(
            value = %configured,
            "PROXY_BASE_URL is not a valid http(s) URL and is ignored for MCP OAuth origin resolution"
        );
        return None;
    };
    let normalized = python_url::join(&parsed.scheme, &parsed.netloc, &parsed.path, "", "");
    Some(normalized.trim_end_matches('/').to_owned())
}

pub fn get_request_base_url(
    proxy_base_url: &str,
    origin: &RequestOrigin<'_>,
) -> Result<String, UrlParseError> {
    if let Some(configured) = normalize_proxy_base_url(proxy_base_url) {
        return Ok(configured);
    }
    let base_url = origin.base_url.trim_end_matches('/');
    let parsed = python_url::parse(base_url)?;
    if !origin.from_trusted_proxy {
        return Ok(base_url.to_owned());
    }
    let scheme = origin
        .x_forwarded_proto
        .filter(|value| !value.is_empty())
        .unwrap_or(parsed.scheme.as_str());
    let netloc = if let Some(host) = origin.x_forwarded_host.filter(|value| !value.is_empty()) {
        let port = origin.x_forwarded_port.filter(|value| !value.is_empty());
        if host.contains(':') && !host.starts_with('[') {
            host.to_owned()
        } else {
            port.map_or_else(
                || host.to_owned(),
                |forwarded| format!("{host}:{forwarded}"),
            )
        }
    } else {
        let mut netloc = parsed.netloc.clone();
        if let Some(port) = origin.x_forwarded_port.filter(|value| !value.is_empty())
            && !netloc.contains(':')
        {
            netloc = format!("{netloc}:{port}");
        }
        netloc
    };
    Ok(python_url::join(
        scheme,
        &strip_default_port(scheme, &netloc),
        &parsed.path,
        "",
        "",
    ))
}

fn strip_ipv6_default_port(lowered: &str, default_port: Option<i128>) -> String {
    let Some(close) = lowered.rfind(']') else {
        return lowered.to_owned();
    };
    let (closed, tail) = lowered.split_at(close);
    let port = tail
        .strip_prefix(']')
        .and_then(|remainder| remainder.strip_prefix(':'))
        .and_then(python_url::netloc::int_value);
    if default_port.is_some_and(|default| port == Some(default)) {
        return format!("{closed}]");
    }
    lowered.to_owned()
}
