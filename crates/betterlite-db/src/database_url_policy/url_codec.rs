use std::fmt::Write as _;
use std::str::from_utf8;

// Mirrors urllib.parse's urlsplit/urlunsplit/parse_qsl/urlencode for postgres/postgresql schemes only; semicolon-params and forced "//" branches never fire and are left out.

pub(super) struct SplitUrl {
    pub(super) scheme: String,
    pub(super) netloc: String,
    pub(super) path: String,
    pub(super) query: String,
    pub(super) fragment: String,
}

// Mirrors the ValueError Python's urlsplit raises for a malformed bracketed netloc.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlSplitError {
    #[error("Invalid IPv6 URL")]
    InvalidIpv6Url,
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
        if starts_alpha && prefix.bytes().all(is_scheme_byte) {
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

// Splits off the "//netloc" prefix, if any, and validates its brackets.
fn split_netloc(rest: &str) -> Result<(String, &str), UrlSplitError> {
    let Some(after) = rest.strip_prefix("//") else {
        return Ok((String::new(), rest));
    };
    let (netloc, tail) = after.find(['/', '?', '#']).map_or((after, ""), |end| {
        after.split_at_checked(end).unwrap_or((after, ""))
    });
    check_netloc_brackets(netloc)?;
    Ok((netloc.to_owned(), tail))
}

// Mirrors _check_bracketed_netloc: an unmatched or misplaced bracket is an error.
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
    let bracketed = hostname_and_port
        .strip_prefix('[')
        .ok_or(UrlSplitError::InvalidIpv6Url)?;
    let (_, after_close) = bracketed
        .split_once(']')
        .ok_or(UrlSplitError::InvalidIpv6Url)?;
    if !after_close.is_empty() && !after_close.starts_with(':') {
        return Err(UrlSplitError::InvalidIpv6Url);
    }
    Ok(())
}

pub(super) fn urlunsplit(parts: &SplitUrl) -> String {
    let mut url = parts.path.clone();
    // Postgres/postgresql never appear in urllib's uses_netloc table, so only the netloc branch applies here.
    if !parts.netloc.is_empty() {
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

const fn is_scheme_byte(byte: u8) -> bool {
    let is_alnum = byte.is_ascii_alphanumeric();
    is_alnum || matches!(byte, b'+' | b'-' | b'.')
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
    let parts: Vec<String> = pairs
        .iter()
        .map(|(name, value)| format!("{}={}", quote_plus(name), quote_plus(value)))
        .collect();
    parts.join("&")
}

// Mirrors urllib.parse.urlencode(groups, doseq=True): repeats a key once per value.
pub(super) fn urlencode_doseq(groups: &[(String, Vec<String>)]) -> String {
    let mut parts = Vec::new();
    for (name, values) in groups {
        let encoded_name = quote_plus(name);
        for value in values {
            parts.push(format!("{encoded_name}={}", quote_plus(value)));
        }
    }
    parts.join("&")
}

const fn is_unreserved_byte(byte: u8) -> bool {
    let is_alnum = byte.is_ascii_alphanumeric();
    is_alnum || matches!(byte, b'_' | b'.' | b'-' | b'~')
}

fn percent_decode_digits(digits: &[u8]) -> Option<u8> {
    let text = from_utf8(digits).ok()?;
    u8::from_str_radix(text, 16).ok()
}

fn percent_encode_byte(byte: u8) -> String {
    if is_unreserved_byte(byte) {
        char::from(byte).to_string()
    } else if byte == b' ' {
        "+".to_owned()
    } else {
        format!("%{byte:02X}")
    }
}

fn quote_plus(text: &str) -> String {
    let bytes = text.as_bytes();
    bytes
        .iter()
        .map(|&byte| percent_encode_byte(byte))
        .collect()
}

fn next_percent_escape(tail: &[u8]) -> Option<(u8, &[u8])> {
    let (digits, after) = tail.split_at_checked(2)?;
    let decoded = percent_decode_digits(digits)?;
    Some((decoded, after))
}

fn unquote_plus(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'+' {
            bytes.push(b' ');
            rest = tail;
        } else if byte == b'%'
            && let Some((decoded, after)) = next_percent_escape(tail)
        {
            bytes.push(decoded);
            rest = after;
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    decode_percent_bytes(bytes)
}

// Replicates bytes.decode("utf-8", errors="replace") without the workspace's banned String::from_utf8_lossy.
fn decode_percent_bytes(bytes: Vec<u8>) -> String {
    let Err(first_error) = from_utf8(&bytes) else {
        return String::from_utf8(bytes).unwrap_or_default();
    };
    let mut text = String::with_capacity(bytes.len());
    let mut rest = bytes.as_slice();
    let mut next_error = Some(first_error.valid_up_to());
    while let Some(valid_len) = next_error {
        let Some((valid, after_valid)) = rest.split_at_checked(valid_len) else {
            break;
        };
        text.push_str(from_utf8(valid).unwrap_or_default());
        text.push(char::REPLACEMENT_CHARACTER);
        let skip = match from_utf8(after_valid) {
            Ok(_) => after_valid.len(),
            Err(error) => error.error_len().unwrap_or(after_valid.len()).max(1),
        };
        rest = after_valid.get(skip..).unwrap_or(&[]);
        next_error = from_utf8(rest).err().map(|error| error.valid_up_to());
    }
    text.push_str(from_utf8(rest).unwrap_or_default());
    text
}
