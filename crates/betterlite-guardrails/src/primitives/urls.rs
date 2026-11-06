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

// Mirrors urllib.parse._check_bracketed_netloc, whose ValueError becomes None here.
fn bracketed_netloc_valid(netloc: &str) -> bool {
    let host = netloc.rsplit_once('@').map_or(netloc, |(_, host)| host);
    let Some((before, rest)) = host.split_once('[') else {
        return !host.contains(']');
    };
    let Some((inside, after)) = rest.split_once(']') else {
        return false;
    };
    if !before.is_empty() || !(after.is_empty() || after.starts_with(':')) {
        return false;
    }
    let Some(future) = inside.strip_prefix('v') else {
        let (address, scope) = inside
            .split_once('%')
            .map_or((inside, true), |(address, scope)| {
                (address, !scope.is_empty() && !scope.contains('%'))
            });
        return scope && address.parse::<Ipv6Addr>().is_ok();
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
