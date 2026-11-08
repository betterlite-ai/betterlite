use betterlite_config::sections::provider_url_destination::pattern::{AllowedDestination, Scheme};

use crate::url_policy::origin::MAX_URL_CHARS;
use crate::user_url::split::urlparse;

// LiteLLM's provider_url_destination_candidates: each comma part, then the text after its first slash
#[must_use]
pub fn candidates(value: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for part in value.split(',') {
        let text = part.trim();
        if !text.is_empty() {
            found.push(text);
        }
        if let Some((_, after)) = text.split_once('/')
            && !after.is_empty()
        {
            found.push(after);
        }
    }
    found
}

// LiteLLM's is_url_destination_allowed_by_host: a configured origin entry, never a suffix or a string prefix
#[must_use]
pub fn destination_allowed(url: &str, allowed: &[AllowedDestination]) -> bool {
    if url.chars().count() > MAX_URL_CHARS {
        return false;
    }
    let Ok(parsed) = urlparse(url, "") else {
        return false;
    };
    if parsed.scheme != "http" && parsed.scheme != "https" {
        return false;
    }
    if parsed.netloc.contains('@') {
        return false;
    }
    let Some(hostname) = parsed.hostname() else {
        return false;
    };
    let folded = hostname.to_lowercase();
    let host = folded.trim_end_matches('.');
    let Ok(explicit_port) = parsed.port() else {
        return false;
    };
    let effective_port = explicit_port.unwrap_or(if parsed.scheme == "https" { 443 } else { 80 });
    allowed.iter().any(|pattern| {
        let scheme_matches = match pattern.scheme {
            Some(Scheme::Http) => parsed.scheme == "http",
            Some(Scheme::Https) => parsed.scheme == "https",
            None => true,
        };
        pattern.host == host
            && scheme_matches
            && pattern
                .port
                .is_none_or(|configured_port| configured_port == effective_port)
    })
}
