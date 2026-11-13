use std::str;

use http::HeaderMap;
use regex::bytes::Regex;

use crate::server_oauth::merge::{ScopesInput, extract_scopes};

const PARAMETER_PATTERN: &str = r#"([a-zA-Z0-9_]+)\s*=\s*"?([^",]+)"?"#;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WwwAuthenticate {
    pub resource_metadata_url: Option<String>,
    pub scopes: Option<Vec<String>>,
}

#[must_use]
pub fn parse_www_authenticate(header: &[u8]) -> WwwAuthenticate {
    let Ok(pattern) = Regex::new(PARAMETER_PATTERN) else {
        return WwwAuthenticate::default();
    };
    let mut resource_metadata_url = None;
    let mut scope = None;
    for section in bearer_sections(header) {
        for capture in pattern.captures_iter(section) {
            let (Some(name), Some(value)) = (capture.get(1), capture.get(2)) else {
                continue;
            };
            let (Ok(name_text), Ok(value_text)) = (
                str::from_utf8(name.as_bytes()),
                str::from_utf8(value.as_bytes()),
            ) else {
                continue;
            };
            if name_text.eq_ignore_ascii_case("resource_metadata") {
                resource_metadata_url = Some(value_text.trim().to_owned());
            }
            if name_text.eq_ignore_ascii_case("scope") {
                scope = Some(value_text.trim().to_owned());
            }
        }
    }
    let scopes = scope.and_then(|text| extract_scopes(ScopesInput::Text(&text)));
    WwwAuthenticate {
        resource_metadata_url,
        scopes,
    }
}

#[must_use]
pub fn challenge_from_headers(headers: &HeaderMap) -> WwwAuthenticate {
    let mut joined = Vec::new();
    for (index, value) in headers.get_all("www-authenticate").iter().enumerate() {
        if index > 0 {
            joined.extend_from_slice(b", ");
        }
        joined.extend_from_slice(value.as_bytes());
    }
    parse_www_authenticate(&joined)
}

fn bearer_sections(header: &[u8]) -> Vec<&[u8]> {
    let mut sections = Vec::new();
    let mut bearer = false;
    for segment in top_level_segments(header.trim_ascii()) {
        let body = segment.trim_ascii();
        if body.is_empty() {
            continue;
        }
        if is_parameter(body) {
            if bearer {
                sections.push(body);
            }
            continue;
        }
        let scheme_end = body
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(body.len());
        let scheme = body.get(..scheme_end).unwrap_or_default();
        bearer = scheme.eq_ignore_ascii_case(b"bearer");
        if bearer {
            let parameters = body.get(scheme_end..).unwrap_or_default().trim_ascii();
            if !parameters.is_empty() {
                sections.push(parameters);
            }
        }
    }
    sections
}

// A comma starts a new challenge unless what follows is an auth-param, so the list is split outside quoted values first.
fn top_level_segments(header: &[u8]) -> Vec<&[u8]> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut index = 0;
    while let Some(byte) = header.get(index) {
        match *byte {
            b'\\' if quoted && !escaped => escaped = true,
            b'"' if !escaped => quoted = !quoted,
            b',' if !quoted && !escaped => {
                segments.push(header.get(start..index).unwrap_or_default());
                start = index.saturating_add(1);
            }
            _ => escaped = false,
        }
        index = index.saturating_add(1);
    }
    segments.push(header.get(start..).unwrap_or_default());
    segments
}

fn is_parameter(segment: &[u8]) -> bool {
    let name_end = segment
        .iter()
        .position(|byte| byte.is_ascii_whitespace() || *byte == b'=')
        .unwrap_or(segment.len());
    segment.get(skip_whitespace(segment, name_end)) == Some(&b'=')
}

fn skip_whitespace(segment: &[u8], start: usize) -> usize {
    let mut index = start;
    while segment.get(index).is_some_and(u8::is_ascii_whitespace) {
        index = index.saturating_add(1);
    }
    index
}
