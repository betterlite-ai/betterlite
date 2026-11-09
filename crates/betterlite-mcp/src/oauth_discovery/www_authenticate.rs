use std::str;

use crate::server_oauth::merge::{ScopesInput, extract_scopes};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WwwAuthenticate {
    pub resource_metadata_url: Option<String>,
    pub scopes: Option<Vec<String>>,
}

#[must_use]
pub fn parse_www_authenticate(header: &[u8]) -> WwwAuthenticate {
    let Some(section) = bearer_parameters(header) else {
        return WwwAuthenticate::default();
    };
    let mut resource_metadata_url = None;
    let mut scope = None;
    for parameter in parameters(section) {
        let (Ok(name), Ok(value)) = (
            str::from_utf8(parameter.name),
            str::from_utf8(parameter.value),
        ) else {
            continue;
        };
        if name.eq_ignore_ascii_case("resource_metadata") {
            resource_metadata_url = Some(value.trim().to_owned());
        }
        if name.eq_ignore_ascii_case("scope") {
            scope = Some(value.trim().to_owned());
        }
    }
    let scopes = scope.and_then(|text| extract_scopes(ScopesInput::Text(&text)));
    WwwAuthenticate {
        resource_metadata_url,
        scopes,
    }
}

fn bearer_parameters(header: &[u8]) -> Option<&[u8]> {
    let trimmed = header.trim_ascii();
    let scheme_end = trimmed
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(trimmed.len());
    let scheme = trimmed.get(..scheme_end)?;
    if !scheme.eq_ignore_ascii_case(b"bearer") {
        return None;
    }
    let rest = trimmed.get(scheme_end..)?;
    Some(rest.trim_ascii())
}

struct Parameter<'section> {
    name: &'section [u8],
    value: &'section [u8],
}

fn parameters(section: &[u8]) -> Vec<Parameter<'_>> {
    let mut found = Vec::new();
    let mut index = 0;
    while index < section.len() {
        match parameter_at(section, index) {
            Some((parameter, next)) => {
                found.push(parameter);
                index = next;
            }
            None => index = index.saturating_add(1),
        }
    }
    found
}

fn parameter_at(section: &[u8], index: usize) -> Option<(Parameter<'_>, usize)> {
    let name_end = identifier_end(section, index)?;
    let after_name = skip_whitespace(section, name_end);
    if section.get(after_name) != Some(&b'=') {
        return None;
    }
    let after_equals = skip_whitespace(section, after_name.saturating_add(1));
    let quoted = section.get(after_equals) == Some(&b'"');
    let value_start = if quoted {
        after_equals.saturating_add(1)
    } else {
        after_equals
    };
    let rest = section.get(value_start..)?;
    let value_length = rest
        .iter()
        .position(|byte| *byte == b'"' || *byte == b',')
        .unwrap_or(rest.len());
    let value_end = value_start.saturating_add(value_length);
    if value_end == value_start {
        return None;
    }
    Some((
        Parameter {
            name: section.get(index..name_end)?,
            value: section.get(value_start..value_end)?,
        },
        value_end,
    ))
}

fn identifier_end(section: &[u8], start: usize) -> Option<usize> {
    let mut end = start;
    while section
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        end = end.saturating_add(1);
    }
    (end > start).then_some(end)
}

fn skip_whitespace(section: &[u8], start: usize) -> usize {
    let mut index = start;
    while section.get(index).is_some_and(u8::is_ascii_whitespace) {
        index = index.saturating_add(1);
    }
    index
}
