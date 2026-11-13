use crate::url_policy::error::UrlPolicyError;
use crate::user_url::split::{Parsed, urlparse};

pub const MAX_URL_CHARS: usize = 0x0001_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

struct Target {
    parsed: Parsed,
    host: String,
}

fn parse_target(url: &str) -> Result<Target, UrlPolicyError> {
    if url.chars().count() > MAX_URL_CHARS {
        return Err(UrlPolicyError::TooLong {
            limit: MAX_URL_CHARS,
        });
    }
    let parsed = urlparse(url, "")?;
    let host = parsed
        .hostname()
        .unwrap_or_default()
        .to_lowercase()
        .trim_end_matches('.')
        .to_owned();
    Ok(Target { parsed, host })
}

impl Origin {
    pub fn parse(url: &str) -> Result<Self, UrlPolicyError> {
        let target = parse_target(url)?;
        let default_port = if target.parsed.scheme == "https" {
            443
        } else {
            80
        };
        let port = target.parsed.port()?.unwrap_or(default_port);
        Ok(Self {
            scheme: target.parsed.scheme,
            host: target.host,
            port,
        })
    }
}

// LiteLLM's assert_same_origin: scheme, normalized host and effective port, with the candidate's scheme checked first
pub fn assert_same_origin(
    candidate_url: &str,
    expected_url: &str,
) -> Result<Origin, UrlPolicyError> {
    let candidate = parse_target(candidate_url)?;
    if candidate.parsed.scheme != "http" && candidate.parsed.scheme != "https" {
        return Err(UrlPolicyError::SchemeNotAllowed);
    }
    let expected = parse_target(expected_url)?;
    if candidate.parsed.scheme != expected.parsed.scheme {
        return Err(UrlPolicyError::OriginScheme);
    }
    if candidate.host.is_empty() || candidate.host != expected.host {
        return Err(UrlPolicyError::OriginHost);
    }
    let default_port = if candidate.parsed.scheme == "https" {
        443
    } else {
        80
    };
    let candidate_port = candidate.parsed.port()?.unwrap_or(default_port);
    let expected_port = expected.parsed.port()?.unwrap_or(default_port);
    if candidate_port != expected_port {
        return Err(UrlPolicyError::OriginPort);
    }
    Ok(Origin {
        scheme: candidate.parsed.scheme,
        host: candidate.host,
        port: candidate_port,
    })
}
