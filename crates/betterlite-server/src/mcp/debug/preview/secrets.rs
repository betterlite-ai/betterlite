use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::iter::once;

use aho_corasick::{AhoCorasickBuilder, MatchKind};
use base64::Engine as _;
use base64::alphabet::STANDARD as BASE64_ALPHABET;
use base64::engine::general_purpose::GeneralPurpose;
use base64::engine::{DecodePaddingMode, GeneralPurposeConfig};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::decode::decode_mcp_stored_contract;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_redact::structured::REDACTED;
use encoding_rs::UTF_8;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use super::cookie::cookie_values;
use super::upstream::{BodyState, UpstreamRequest};
use super::{BODY_CAPTURE_BYTES, FORM_URLENCODED, content_type, content_type_base, header_text};

const SENSITIVE_NAMES: [&str; 2] = ["clientassertion", "code"];
const SENSITIVE_PATTERNS: [&str; 14] = [
    "access",
    "auth",
    "authorization",
    "certificate",
    "cookie",
    "credential",
    "credentials",
    "fingerprint",
    "key",
    "password",
    "private",
    "secret",
    "tenancy",
    "token",
];
const PUBLIC_HEADER_NAMES: [&str; 7] = [
    "accept",
    "accept-encoding",
    "connection",
    "content-length",
    "content-type",
    "host",
    "user-agent",
];
const QUOTE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');
const PLUS_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b' ');
const BASE64_ENGINE: GeneralPurpose = GeneralPurpose::new(
    &BASE64_ALPHABET,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

#[must_use]
pub fn sensitive_field(key: &str) -> bool {
    let normalized: String = key
        .to_lowercase()
        .chars()
        .filter(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        .collect();
    SENSITIVE_NAMES.contains(&normalized.as_str())
        || SENSITIVE_PATTERNS
            .iter()
            .any(|pattern| normalized.contains(*pattern))
}

#[must_use]
pub fn header_secret_values(name: &str, value: &str) -> Vec<String> {
    if name == "cookie" {
        return cookie_values(value).map_or_else(
            || vec![value.to_owned()],
            |values| once(value.to_owned()).chain(values).collect(),
        );
    }
    if !matches!(name, "authorization" | "proxy-authorization") {
        return vec![value.to_owned()];
    }
    let (scheme, credential) = value.split_once(' ').unwrap_or((value, ""));
    if !scheme.eq_ignore_ascii_case("basic") {
        return vec![value.to_owned(), credential.to_owned()];
    }
    let Some(values) = basic_credential_values(credential) else {
        return vec![value.to_owned(), credential.to_owned()];
    };
    let mut secrets = vec![value.to_owned()];
    secrets.extend(values);
    secrets
}

fn basic_credential_values(credential: &str) -> Option<Vec<String>> {
    let decoded = String::from_utf8(BASE64_ENGINE.decode(credential).ok()?).ok()?;
    let password = decoded
        .split_once(':')
        .map_or_else(String::new, |(_user, password)| password.to_owned());
    let unescaped = unquote_plus(&password);
    Some(vec![credential.to_owned(), decoded, password, unescaped])
}

#[must_use]
pub fn body_secret_values(request: &UpstreamRequest) -> Option<Vec<String>> {
    let BodyState::Read(raw) = &request.body else {
        return None;
    };
    if raw.is_empty() {
        return Some(Vec::new());
    }
    if raw.len() > BODY_CAPTURE_BYTES {
        return None;
    }
    if content_type_base(&content_type(&request.headers)) == FORM_URLENCODED {
        let text = UTF_8.decode_without_bom_handling(raw).0;
        return Some(
            parse_qsl(&text, false)
                .into_iter()
                .filter(|(key, _value)| sensitive_field(key))
                .map(|(_key, value)| value)
                .collect(),
        );
    }
    let Ok(body) = decode_mcp_stored_contract(raw) else {
        return None;
    };
    let mut values = Vec::new();
    collect_secret_leaves(&body, 0, false, &mut values);
    Some(values)
}

fn collect_secret_leaves(
    body: &JsonValue,
    depth: usize,
    sensitive: bool,
    values: &mut Vec<String>,
) {
    match body {
        JsonValue::String(text) => {
            if sensitive || depth == 0 {
                values.push(text.clone());
            }
        }
        JsonValue::Object(fields) => {
            for (key, value) in fields {
                collect_secret_leaves(
                    value,
                    depth.saturating_add(1),
                    sensitive || sensitive_field(key),
                    values,
                );
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_secret_leaves(item, depth.saturating_add(1), sensitive, values);
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {}
    }
}

#[must_use]
pub fn request_secret_values(request: &UpstreamRequest) -> Option<Vec<String>> {
    let mut values = body_secret_values(request)?;
    values.push(url_password(&request.url));
    values.extend(query_values(&request.url));
    for (name, value) in &request.headers {
        if PUBLIC_HEADER_NAMES.contains(&name.as_str()) {
            continue;
        }
        values.extend(header_secret_values(name.as_str(), &header_text(value)));
    }
    Some(sorted_secrets(values))
}

fn url_password(url: &HttpxUrl) -> String {
    url.userinfo
        .split_once(':')
        .map_or_else(String::new, |(_user, password)| {
            percent_decode_str(password)
                .decode_utf8_lossy()
                .into_owned()
        })
}

fn query_values(url: &HttpxUrl) -> Vec<String> {
    url.query.as_deref().map_or_else(Vec::new, |query| {
        parse_qsl(query, true)
            .into_iter()
            .map(|(_name, value)| value)
            .collect()
    })
}

fn sorted_secrets(values: Vec<String>) -> Vec<String> {
    let unique: BTreeSet<String> = values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect();
    let mut sorted: Vec<String> = unique.into_iter().collect();
    sorted.sort_by_key(|value| Reverse(value.chars().count()));
    sorted
}

#[must_use]
pub fn mask_known_values(text: &str, secrets: &[String]) -> String {
    let mut variants: BTreeSet<String> = BTreeSet::new();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        variants.insert(secret.clone());
        variants.insert(json_escaped(secret));
        variants.insert(utf8_percent_encode(secret, QUOTE_SET).to_string());
        variants.insert(plus_encode(secret));
    }
    if variants.is_empty() {
        return text.to_owned();
    }
    let mut patterns: Vec<String> = variants.into_iter().collect();
    patterns.sort_by_key(|pattern| Reverse(pattern.chars().count()));
    let Ok(matcher) = AhoCorasickBuilder::new()
        .match_kind(MatchKind::LeftmostLongest)
        .build(&patterns)
    else {
        return REDACTED.to_owned();
    };
    let mut masked = String::with_capacity(text.len());
    let replaced =
        matcher.try_replace_all_with(text, &mut masked, |_matched, _haystack, destination| {
            destination.push_str(REDACTED);
            true
        });
    if replaced.is_err() {
        return REDACTED.to_owned();
    }
    masked
}

fn json_escaped(secret: &str) -> String {
    let Ok(quoted) = serde_json::to_string(secret) else {
        return secret.to_owned();
    };
    quoted
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map_or_else(|| secret.to_owned(), str::to_owned)
}

pub(super) fn plus_encode(text: &str) -> String {
    utf8_percent_encode(text, PLUS_SET)
        .to_string()
        .replace(' ', "+")
}

fn unquote_plus(text: &str) -> String {
    let unescaped = text.replace('+', " ");
    percent_decode_str(&unescaped)
        .decode_utf8_lossy()
        .into_owned()
}

pub(super) fn parse_qsl(text: &str, keep_blank_values: bool) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for field in text.split('&') {
        if field.is_empty() {
            continue;
        }
        let split = field.split_once('=');
        if split.is_none() && !keep_blank_values {
            continue;
        }
        let (name, value) = split.unwrap_or((field, ""));
        if value.is_empty() && !keep_blank_values {
            continue;
        }
        pairs.push((unquote_plus(name), unquote_plus(value)));
    }
    pairs
}
