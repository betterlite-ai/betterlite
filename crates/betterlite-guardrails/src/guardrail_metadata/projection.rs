use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{
    APPLIED_GUARDRAILS_KEY, APPLIED_POLICIES_KEY, GUARDRAIL_SCAN_IDS_METADATA_KEY,
    GUARDRAIL_SCAN_METADATA_METADATA_KEY, GuardrailMetadataError,
    MAX_GUARDRAIL_SCAN_METADATA_HEADER_LENGTH, PILLAR_FLAGGED_KEY, PILLAR_RESPONSE_HEADERS_KEY,
    POLICY_SOURCES_KEY, SEMANTIC_SIMILARITY_KEY, TRUSTED_PILLAR_HEADERS_KEY,
    serialize_scan_metadata_header,
};

pub(super) type Metadata<'data> = BTreeMap<&'data str, &'data GuardrailValue>;

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

fn text_list(
    value: Option<&GuardrailValue>,
    field: &'static str,
) -> Result<Option<Vec<String>>, GuardrailMetadataError> {
    match value {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Array(items)) => {
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
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Object(_),
        ) => Err(GuardrailMetadataError::NotTyped {
            field,
            expected: "a list of strings",
        }),
    }
}

// LiteLLM emits the joined header whenever the list key is present, an explicit empty list included
fn joined_header(
    merged: &Metadata<'_>,
    key: &'static str,
    header: &'static str,
) -> Result<Option<(String, String)>, GuardrailMetadataError> {
    match text_list(merged.get(key), key)? {
        None => Ok(None),
        Some(names) => Ok(Some((header.to_owned(), names.join(",")))),
    }
}

pub(super) fn applied_guardrails_header(
    merged: &Metadata<'_>,
) -> Result<Option<(String, String)>, GuardrailMetadataError> {
    joined_header(
        merged,
        APPLIED_GUARDRAILS_KEY,
        "x-litellm-applied-guardrails",
    )
}

pub(super) fn applied_policies_header(
    merged: &Metadata<'_>,
) -> Result<Option<(String, String)>, GuardrailMetadataError> {
    joined_header(merged, APPLIED_POLICIES_KEY, "x-litellm-applied-policies")
}

pub(super) fn scan_ids_header(
    merged: &Metadata<'_>,
) -> Result<Option<(String, String)>, GuardrailMetadataError> {
    match merged.get(GUARDRAIL_SCAN_IDS_METADATA_KEY) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Array(items)) if items.is_empty() => Ok(None),
        Some(GuardrailValue::Array(_)) => {
            let value = merged.get(GUARDRAIL_SCAN_IDS_METADATA_KEY);
            let Some(names) = text_list(value, GUARDRAIL_SCAN_IDS_METADATA_KEY)? else {
                return Ok(None);
            };
            Ok(Some((
                "x-litellm-guardrail-scan-id".to_owned(),
                names.join(","),
            )))
        }
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Object(_),
        ) => Err(GuardrailMetadataError::NotTyped {
            field: GUARDRAIL_SCAN_IDS_METADATA_KEY,
            expected: "a list of strings",
        }),
    }
}

pub(super) fn scan_metadata_header(
    merged: &Metadata<'_>,
) -> Result<Option<(String, String)>, GuardrailMetadataError> {
    let Some(GuardrailValue::Array(entries)) = merged.get(GUARDRAIL_SCAN_METADATA_METADATA_KEY)
    else {
        return Ok(None);
    };
    let Some(value) =
        serialize_scan_metadata_header(entries, MAX_GUARDRAIL_SCAN_METADATA_HEADER_LENGTH)?
    else {
        return Ok(None);
    };
    Ok(Some((
        "x-litellm-guardrail-scan-metadata".to_owned(),
        value,
    )))
}

// the metadata bucket is a BTreeMap, so sources come out in key order
pub(super) fn policy_sources_header(
    merged: &Metadata<'_>,
) -> Result<Option<(String, String)>, GuardrailMetadataError> {
    match merged.get(POLICY_SOURCES_KEY) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Object(sources)) => {
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
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_),
        ) => Err(GuardrailMetadataError::NotTyped {
            field: POLICY_SOURCES_KEY,
            expected: "a map of strings",
        }),
    }
}

pub(super) fn semantic_similarity_header(
    merged: &Metadata<'_>,
) -> Result<Option<(String, String)>, GuardrailMetadataError> {
    let header = "x-litellm-semantic-similarity".to_owned();
    match merged.get(SEMANTIC_SIMILARITY_KEY) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Decimal(number)) => {
            let text = serde_json::Number::from_f64(*number)
                .ok_or(GuardrailMetadataError::NotTyped {
                    field: SEMANTIC_SIMILARITY_KEY,
                    expected: "a finite number",
                })?
                .to_string();
            Ok(Some((header, text)))
        }
        Some(GuardrailValue::SignedInteger(number)) => Ok(Some((header, number.to_string()))),
        Some(GuardrailValue::UnsignedInteger(number)) => Ok(Some((header, number.to_string()))),
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        ) => Err(GuardrailMetadataError::NotTyped {
            field: SEMANTIC_SIMILARITY_KEY,
            expected: "a number",
        }),
    }
}

fn flagged_header(merged: &Metadata<'_>) -> Result<Vec<(String, String)>, GuardrailMetadataError> {
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

pub(super) fn pillar_headers(
    merged: &Metadata<'_>,
) -> Result<Vec<(String, String)>, GuardrailMetadataError> {
    let trusted = matches!(
        merged.get(TRUSTED_PILLAR_HEADERS_KEY),
        Some(GuardrailValue::Boolean(true))
    );
    if !trusted {
        return Ok(Vec::new());
    }
    let Some(GuardrailValue::Object(pillar)) = merged.get(PILLAR_RESPONSE_HEADERS_KEY) else {
        return flagged_header(merged);
    };
    let mut headers = Vec::new();
    for (name, value) in pillar {
        if !name.to_lowercase().starts_with("x-pillar-") {
            continue;
        }
        let GuardrailValue::Text(text) = value else {
            let GuardrailValue::Null = value else {
                return Err(GuardrailMetadataError::NotTyped {
                    field: PILLAR_RESPONSE_HEADERS_KEY,
                    expected: "a map of strings",
                });
            };
            continue;
        };
        headers.push((name.clone(), text.clone()));
    }
    Ok(headers)
}

pub(super) fn push_optional(headers: &mut Vec<(String, String)>, header: Option<(String, String)>) {
    if let Some(found) = header {
        headers.push(found);
    }
}
