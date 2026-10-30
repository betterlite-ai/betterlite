use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode, utf8_percent_encode};
use std::fmt::Write as _;
use std::net::{Ipv4Addr, Ipv6Addr};
use unicode_normalization::UnicodeNormalization as _;

// Mirrors CPython's quote_plus: alphanumerics and _.-~ pass through literally, everything else is percent-escaped.
const QUOTE_PLUS_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b' ');

pub(super) struct SplitUrl {
    pub(super) scheme: String,
    pub(super) netloc: String,
    pub(super) path: String,
    pub(super) query: String,
    pub(super) fragment: String,
}

// Mirrors the ValueError Python's urlsplit raises for a malformed or invalid bracketed netloc.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlSplitError {
    #[error("Invalid IPv6 URL")]
    InvalidIpv6Url,
    #[error("'{hostname}' does not appear to be an IPv4 or IPv6 address")]
    NotIpAddress { hostname: String },
    #[error("An IPv4 address cannot be in brackets")]
    Ipv4AddressInBrackets,
    #[error("IPvFuture address is invalid")]
    InvalidIpvFuture,
    #[error("netloc contains invalid characters under NFKC normalization")]
    InvalidNfkcNetloc,
}

pub(super) fn urlsplit(url: &str) -> Result<SplitUrl, UrlSplitError> {
    let trimmed = url.trim_start_matches(|character: char| u32::from(character) <= 0x20);
    let cleaned: String = trimmed
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();
    let mut rest = cleaned.as_str();
    let mut scheme = String::new();
    if let Some((prefix, remainder)) = rest.split_once(':') {
        let starts_alpha = prefix.starts_with(|character: char| character.is_ascii_alphabetic());
        if starts_alpha
            && prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        {
            scheme = prefix.to_ascii_lowercase();
            rest = remainder;
        }
    }
    let (netloc, after_netloc) = split_netloc(rest)?;
    let (before_fragment, fragment) = after_netloc.split_once('#').unwrap_or((after_netloc, ""));
    let (path, query) = before_fragment
        .split_once('?')
        .unwrap_or((before_fragment, ""));
    Ok(SplitUrl {
        scheme,
        netloc,
        path: path.to_owned(),
        query: query.to_owned(),
        fragment: fragment.to_owned(),
    })
}

fn split_netloc(rest: &str) -> Result<(String, &str), UrlSplitError> {
    let Some(after) = rest.strip_prefix("//") else {
        return Ok((String::new(), rest));
    };
    let (netloc, tail) = after.find(['/', '?', '#']).map_or((after, ""), |end| {
        after.split_at_checked(end).unwrap_or((after, ""))
    });
    check_netloc_brackets(netloc)?;
    check_netloc_nfkc(netloc)?;
    Ok((netloc.to_owned(), tail))
}

// Mirrors _checknetloc: a non-ASCII netloc that NFKC-normalizes into one of '/?#@:' is a homoglyph attack.
fn check_netloc_nfkc(netloc: &str) -> Result<(), UrlSplitError> {
    if netloc.is_ascii() {
        return Ok(());
    }
    let stripped: String = netloc
        .chars()
        .filter(|character| !matches!(character, '@' | ':' | '#' | '?'))
        .collect();
    let normalized: String = stripped.nfkc().collect();
    let smuggled_a_separator =
        stripped != normalized && normalized.contains(['/', '?', '#', '@', ':']);
    if smuggled_a_separator {
        Err(UrlSplitError::InvalidNfkcNetloc)
    } else {
        Ok(())
    }
}

// Mirrors _check_bracketed_netloc: an unmatched, misplaced or invalid-host bracket is an error.
fn check_netloc_brackets(netloc: &str) -> Result<(), UrlSplitError> {
    let has_open = netloc.contains('[');
    let has_close = netloc.contains(']');
    if has_open != has_close {
        return Err(UrlSplitError::InvalidIpv6Url);
    }
    if !has_open {
        return Ok(());
    }
    let hostname_and_port = netloc.rsplit_once('@').map_or(netloc, |(_, rest)| rest);
    let hostname = if hostname_and_port.contains('[') {
        let after_open = hostname_and_port
            .strip_prefix('[')
            .ok_or(UrlSplitError::InvalidIpv6Url)?;
        let (hostname, after_close) = after_open.split_once(']').unwrap_or((after_open, ""));
        if !after_close.is_empty() && !after_close.starts_with(':') {
            return Err(UrlSplitError::InvalidIpv6Url);
        }
        hostname
    } else {
        hostname_and_port
            .split_once(':')
            .map_or(hostname_and_port, |(host, _)| host)
    };
    check_bracketed_host(hostname)
}

// Mirrors _check_bracketed_host: the bracketed host must be an IPvFuture spelling or a non-IPv4 address.
fn check_bracketed_host(hostname: &str) -> Result<(), UrlSplitError> {
    if let Some(rest) = hostname.strip_prefix(['v', 'V']) {
        return if is_ipvfuture_rest(rest) {
            Ok(())
        } else {
            Err(UrlSplitError::InvalidIpvFuture)
        };
    }
    if hostname.parse::<Ipv4Addr>().is_ok() {
        return Err(UrlSplitError::Ipv4AddressInBrackets);
    }
    if is_ipv6_with_optional_zone(hostname) {
        return Ok(());
    }
    Err(UrlSplitError::NotIpAddress {
        hostname: hostname.to_owned(),
    })
}

// Mirrors ipaddress.IPv6Address._split_scope_id: a %zone suffix is kept raw, never empty, never doubled.
fn is_ipv6_with_optional_zone(hostname: &str) -> bool {
    match hostname.split_once('%') {
        Some((address, zone)) => {
            !zone.is_empty() && !zone.contains('%') && address.parse::<Ipv6Addr>().is_ok()
        }
        None => hostname.parse::<Ipv6Addr>().is_ok(),
    }
}

// Matches r"\Av[a-fA-F0-9]+\..+\Z" against "v" + rest, with the leading 'v' already stripped.
fn is_ipvfuture_rest(rest: &str) -> bool {
    let Some(dot_index) = rest.find('.') else {
        return false;
    };
    let (hex_digits, after_dot) = rest.split_at(dot_index);
    !hex_digits.is_empty()
        && hex_digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        && after_dot.len() > 1
}

pub(super) fn urlunsplit(parts: &SplitUrl) -> String {
    let mut url = parts.path.clone();
    if parts.netloc.is_empty() {
        // An empty netloc with a path starting "//" (postgresql:////db) still forces a leading "//".
        if url.starts_with("//") {
            url = format!("//{url}");
        }
    } else {
        if !url.is_empty() && !url.starts_with('/') {
            url.insert(0, '/');
        }
        url = format!("//{}{url}", parts.netloc);
    }
    if !parts.scheme.is_empty() {
        url = format!("{}:{url}", parts.scheme);
    }
    if !parts.query.is_empty() && write!(url, "?{}", parts.query).is_err() {
        return url;
    }
    if !parts.fragment.is_empty() && write!(url, "#{}", parts.fragment).is_err() {
        return url;
    }
    url
}

// Mirrors urllib.parse.parse_qsl(query, keep_blank_values=keep_blank_values).
pub(super) fn parse_qsl(query: &str, keep_blank_values: bool) -> Vec<(String, String)> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut pairs = Vec::new();
    for chunk in query.split('&') {
        if chunk.is_empty() {
            continue;
        }
        let (name, value) = chunk.split_once('=').unwrap_or((chunk, ""));
        if value.is_empty() && !keep_blank_values {
            continue;
        }
        pairs.push((unquote_plus(name), unquote_plus(value)));
    }
    pairs
}

// Mirrors urllib.parse.parse_qs: parse_qsl grouped by key, values kept in order, blanks dropped.
pub(super) fn parse_qs(query: &str) -> Vec<(String, Vec<String>)> {
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for (name, value) in parse_qsl(query, false) {
        match grouped.iter_mut().find(|(key, _)| *key == name) {
            Some((_, values)) => values.push(value),
            None => grouped.push((name, vec![value])),
        }
    }
    grouped
}

// Mirrors urllib.parse.urlencode(pairs) with no doseq.
pub(super) fn urlencode_pairs(pairs: &[(String, String)]) -> String {
    let mut parts = Vec::new();
    for (name, value) in pairs {
        let encoded_name = utf8_percent_encode(name, QUOTE_PLUS_SET)
            .to_string()
            .replace(' ', "+");
        let encoded_value = utf8_percent_encode(value, QUOTE_PLUS_SET)
            .to_string()
            .replace(' ', "+");
        parts.push(format!("{encoded_name}={encoded_value}"));
    }
    parts.join("&")
}

// Mirrors urllib.parse.urlencode(groups, doseq=True): repeats a key once per value.
pub(super) fn urlencode_doseq(groups: &[(String, Vec<String>)]) -> String {
    let mut parts = Vec::new();
    for (name, values) in groups {
        let encoded_name = utf8_percent_encode(name, QUOTE_PLUS_SET)
            .to_string()
            .replace(' ', "+");
        for value in values {
            let encoded_value = utf8_percent_encode(value, QUOTE_PLUS_SET)
                .to_string()
                .replace(' ', "+");
            parts.push(format!("{encoded_name}={encoded_value}"));
        }
    }
    parts.join("&")
}

// Matches urllib.parse.unquote_plus: percent_decode leaves a bad escape as a literal '%', like CPython.
fn unquote_plus(text: &str) -> String {
    let space_separated = text.replace('+', " ");
    let decoded: Vec<u8> = percent_decode(space_separated.as_bytes()).collect();
    decode_percent_bytes(&decoded)
}

// Replicates bytes.decode("utf-8", errors="replace") using the standard library's own chunking.
fn decode_percent_bytes(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
