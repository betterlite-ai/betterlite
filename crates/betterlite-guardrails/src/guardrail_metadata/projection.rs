use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{
    GuardrailMetadataError, MAX_SCAN_METADATA_LENGTH, PILLAR_FLAGGED_KEY,
    PILLAR_RESPONSE_HEADERS_KEY, POLICY_SOURCES_KEY, SCAN_METADATA_KEY, SEMANTIC_SIMILARITY_KEY,
    TRUSTED_PILLAR_HEADERS_KEY, serialize_scan_metadata_header,
};

// serde_json's arbitrary_precision feature hands a JSON decimal to an untagged enum as this one-key map
const NUMBER_KEY: &str = "$serde_json::private::Number";

pub(super) type Metadata<'data> = BTreeMap<&'data str, &'data GuardrailValue>;
pub(super) type Header = (String, String);
pub(super) type MetadataResult<T> = Result<T, GuardrailMetadataError>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EmptyList {
    Emit,
    Omit,
}

pub(super) fn merged_metadata(request_data: &BTreeMap<String, GuardrailValue>) -> Metadata<'_> {
    let mut merged = BTreeMap::new();
    for location in ["metadata", "litellm_metadata"] {
        let Some(GuardrailValue::Object(fields)) = request_data.get(location) else {
            continue;
        };
        for (key, value) in fields {
            merged.insert(key.as_str(), value);
        }
    }
    merged
}

// LiteLLM's isinstance guard on the values a header can hold: a null is the field's default
const fn absent_or(
    value: &GuardrailValue,
    field: &'static str,
    expected: &'static str,
) -> MetadataResult<()> {
    if matches!(value, GuardrailValue::Null) {
        return Ok(());
    }
    Err(GuardrailMetadataError::NotTyped { field, expected })
}

fn text_list(
    value: Option<&GuardrailValue>,
    field: &'static str,
) -> MetadataResult<Option<Vec<String>>> {
    let Some(found) = value else {
        return Ok(None);
    };
    let GuardrailValue::Array(items) = found else {
        absent_or(found, field, "a list of strings")?;
        return Ok(None);
    };
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let GuardrailValue::Text(name) = item else {
            return Err(GuardrailMetadataError::NotTyped {
                field,
                expected: "a list of strings",
            });
        };
        names.push(name.clone());
    }
    Ok(Some(names))
}

// LiteLLM emits the joined header whenever the list key is present, except for the scan ids header
pub(super) fn joined_header(
    merged: &Metadata<'_>,
    key: &'static str,
    header: &'static str,
    empty: EmptyList,
) -> MetadataResult<Option<Header>> {
    let Some(names) = text_list(merged.get(key).copied(), key)? else {
        return Ok(None);
    };
    if names.is_empty() && empty == EmptyList::Omit {
        return Ok(None);
    }
    Ok(Some((header.to_owned(), names.join(","))))
}

fn scan_metadata_value(merged: &Metadata<'_>) -> MetadataResult<Option<String>> {
    let Some(GuardrailValue::Array(entries)) = merged.get(SCAN_METADATA_KEY) else {
        return Ok(None);
    };
    serialize_scan_metadata_header(entries, MAX_SCAN_METADATA_LENGTH)
}

pub(super) fn scan_metadata_header(merged: &Metadata<'_>) -> MetadataResult<Option<Header>> {
    let Some(value) = scan_metadata_value(merged)? else {
        return Ok(None);
    };
    Ok(Some((
        "x-litellm-guardrail-scan-metadata".to_owned(),
        value,
    )))
}

// the metadata bucket is a BTreeMap, so sources come out in key order
pub(super) fn policy_sources_header(merged: &Metadata<'_>) -> MetadataResult<Option<Header>> {
    let Some(value) = merged.get(POLICY_SOURCES_KEY) else {
        return Ok(None);
    };
    let GuardrailValue::Object(sources) = value else {
        absent_or(value, POLICY_SOURCES_KEY, "a map of strings")?;
        return Ok(None);
    };
    if sources.is_empty() {
        return Ok(None);
    }
    let mut parts = Vec::with_capacity(sources.len());
    for (name, reason) in sources {
        let GuardrailValue::Text(text) = reason else {
            return Err(GuardrailMetadataError::NotTyped {
                field: POLICY_SOURCES_KEY,
                expected: "a map of strings",
            });
        };
        parts.push(format!("{name}={text}"));
    }
    Ok(Some((
        "x-litellm-policy-sources".to_owned(),
        parts.join("; "),
    )))
}

fn number_text(number: f64, field: &'static str) -> MetadataResult<String> {
    serde_json::Number::from_f64(number)
        .map(|found| found.to_string())
        .ok_or(GuardrailMetadataError::NotTyped {
            field,
            expected: "a finite number",
        })
}

fn similarity_text(merged: &Metadata<'_>) -> MetadataResult<Option<String>> {
    let Some(value) = merged.get(SEMANTIC_SIMILARITY_KEY) else {
        return Ok(None);
    };
    let literal = match value {
        GuardrailValue::Null => return Ok(None),
        GuardrailValue::Decimal(number) => {
            return Ok(Some(number_text(*number, SEMANTIC_SIMILARITY_KEY)?));
        }
        GuardrailValue::SignedInteger(number) => return Ok(Some(number.to_string())),
        GuardrailValue::UnsignedInteger(number) => return Ok(Some(number.to_string())),
        GuardrailValue::Object(fields) if fields.len() == 1 => fields.get(NUMBER_KEY),
        GuardrailValue::Boolean(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => None,
    };
    let parsed = if let Some(GuardrailValue::Text(text)) = literal {
        text.parse::<f64>().ok()
    } else {
        None
    };
    let Some(number) = parsed else {
        return Err(GuardrailMetadataError::NotTyped {
            field: SEMANTIC_SIMILARITY_KEY,
            expected: "a number",
        });
    };
    Ok(Some(number_text(number, SEMANTIC_SIMILARITY_KEY)?))
}

pub(super) fn semantic_similarity_header(merged: &Metadata<'_>) -> MetadataResult<Option<Header>> {
    let Some(text) = similarity_text(merged)? else {
        return Ok(None);
    };
    Ok(Some(("x-litellm-semantic-similarity".to_owned(), text)))
}

// the trusted map is absent when there is no object, so pillar_flagged supplies the only header
fn flagged_headers(merged: &Metadata<'_>) -> MetadataResult<Vec<Header>> {
    match merged.get(PILLAR_FLAGGED_KEY) {
        None | Some(GuardrailValue::Null) => Ok(Vec::new()),
        Some(GuardrailValue::Boolean(flagged)) => {
            Ok(vec![("x-pillar-flagged".to_owned(), flagged.to_string())])
        }
        Some(_) => Err(GuardrailMetadataError::NotTyped {
            field: PILLAR_FLAGGED_KEY,
            expected: "a boolean",
        }),
    }
}

pub(super) fn pillar_headers(merged: &Metadata<'_>) -> MetadataResult<Vec<Header>> {
    let trusted = matches!(
        merged.get(TRUSTED_PILLAR_HEADERS_KEY),
        Some(GuardrailValue::Boolean(true))
    );
    if !trusted {
        return Ok(Vec::new());
    }
    let Some(GuardrailValue::Object(pillar)) = merged.get(PILLAR_RESPONSE_HEADERS_KEY) else {
        return flagged_headers(merged);
    };
    let mut headers = Vec::new();
    for (name, value) in pillar {
        if !name.to_lowercase().starts_with("x-pillar-") {
            continue;
        }
        let GuardrailValue::Text(text) = value else {
            absent_or(value, PILLAR_RESPONSE_HEADERS_KEY, "a map of strings")?;
            continue;
        };
        headers.push((name.clone(), text.clone()));
    }
    Ok(headers)
}

pub(super) fn push_optional(headers: &mut Vec<Header>, header: Option<Header>) {
    if let Some(found) = header {
        headers.push(found);
    }
}
