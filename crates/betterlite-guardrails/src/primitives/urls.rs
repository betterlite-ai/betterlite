use std::net::Ipv6Addr;

use regex::{Regex, RegexBuilder};

use super::error::PrimitiveError;

const URL_PATTERN: &str = r"https?://(?:[-\w.]|(?:%[\da-fA-F]{2}))+[^\s]*";

pub struct UrlExtractor {
    pattern: Regex,
}

impl UrlExtractor {
    pub fn new() -> Result<Self, PrimitiveError> {
        let pattern = RegexBuilder::new(URL_PATTERN)
            .case_insensitive(true)
            .build()
            .map_err(|source| PrimitiveError::UrlPattern { source })?;
        Ok(Self { pattern })
    }

    #[must_use]
    pub fn extract_urls(&self, text: &str) -> Vec<String> {
        self.pattern
            .find_iter(text)
            .map(|found| found.as_str().to_owned())
            .collect()
    }

    #[must_use]
    pub fn all_urls_valid(&self, text: &str) -> bool {
        self.pattern
            .find_iter(text)
            .all(|found| is_valid_url(found.as_str()))
    }
}

#[must_use]
pub fn is_valid_url(url: &str) -> bool {
    let Some((scheme, netloc)) = split_url(url) else {
        return false;
    };
    !scheme.is_empty() && !netloc.is_empty()
}

#[must_use]
pub fn get_url_domain(url: &str) -> Option<String> {
    split_url(url)
        .map(|(_, netloc)| netloc)
        .filter(|netloc| !netloc.is_empty())
}

// Splits like urllib.parse.urlparse; None is the ValueError urlparse raises for a broken IPv6 host.
fn split_url(url: &str) -> Option<(String, String)> {
    let cleaned: String = url
        .trim_start_matches(|character: char| character <= ' ')
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();
    let (scheme, rest) = match cleaned.split_once(':') {
        Some((candidate, after)) if is_scheme(candidate) => (candidate.to_lowercase(), after),
        Some(_) | None => (String::new(), cleaned.as_str()),
    };
    let netloc = rest.strip_prefix("//").map_or("", |authority| {
        authority.split(['/', '?', '#']).next().unwrap_or_default()
    });
    if !bracketed_netloc_valid(netloc) {
        return None;
    }
    Some((scheme, netloc.to_owned()))
}

// Mirrors urlsplit's bracket checks and urllib.parse._check_bracketed_netloc, whose ValueErrors become false.
fn bracketed_netloc_valid(netloc: &str) -> bool {
    let has_open = netloc.contains('[');
    let has_close = netloc.contains(']');
    if has_open != has_close {
        return false;
    }
    if !has_open {
        return true;
    }
    let host_and_port = netloc.rsplit_once('@').map_or(netloc, |(_, host)| host);
    let Some((before, bracketed)) = host_and_port.split_once('[') else {
        let hostname = host_and_port
            .split_once(':')
            .map_or(host_and_port, |(hostname, _)| hostname);
        return ipvfuture_valid(hostname);
    };
    if !before.is_empty() {
        return false;
    }
    let (hostname, port) = bracketed
        .split_once(']')
        .map_or((bracketed, ""), |(hostname, port)| (hostname, port));
    (port.is_empty() || port.starts_with(':')) && bracketed_host_valid(hostname)
}

fn bracketed_host_valid(hostname: &str) -> bool {
    if hostname.starts_with('v') {
        return ipvfuture_valid(hostname);
    }
    let (address, scope) = hostname
        .split_once('%')
        .map_or((hostname, true), |(address, scope)| {
            (address, !scope.is_empty() && !scope.contains('%'))
        });
    scope && address.parse::<Ipv6Addr>().is_ok()
}

// urllib.parse's IPvFuture form: \Av[a-fA-F0-9]+\..+\Z.
fn ipvfuture_valid(hostname: &str) -> bool {
    let Some(future) = hostname.strip_prefix('v') else {
        return false;
    };
    future.split_once('.').is_some_and(|(digits, tail)| {
        !digits.is_empty()
            && digits
                .chars()
                .all(|character| character.is_ascii_hexdigit())
            && !tail.is_empty()
    })
}

fn is_scheme(candidate: &str) -> bool {
    candidate.starts_with(|first: char| first.is_ascii_alphabetic())
        && candidate.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}
