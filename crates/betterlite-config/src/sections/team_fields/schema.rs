use std::collections::BTreeSet;

use thiserror::Error;

use crate::sections::settings::JsonValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataSchemaField {
    pub key: String,
    pub label: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SchemaError {
    #[error("team_metadata_schema must be a list of metadata field objects")]
    NotAList,
    #[error("team_metadata_schema[{index}]: {message}")]
    Entry { index: usize, message: &'static str },
    #[error("team_metadata_schema[{index}].{field}: {message}")]
    Field {
        index: usize,
        field: &'static str,
        message: &'static str,
    },
    #[error("team_metadata_schema contains duplicate keys: {keys}")]
    DuplicateKeys { keys: String },
}

pub fn parse_team_metadata_schema(
    raw: Option<&JsonValue>,
) -> Result<Vec<MetadataSchemaField>, SchemaError> {
    let Some(value) = raw.filter(|found| !found.is_null()) else {
        return Ok(Vec::new());
    };
    let JsonValue::Array(items) = value else {
        return Err(SchemaError::NotAList);
    };
    let fields = items
        .iter()
        .enumerate()
        .map(|(index, item)| parse_field(index, item))
        .collect::<Result<Vec<_>, _>>()?;
    let duplicates = duplicate_keys(&fields);
    if !duplicates.is_empty() {
        return Err(SchemaError::DuplicateKeys {
            keys: duplicates.join(", "),
        });
    }
    Ok(fields)
}

fn parse_field(index: usize, value: &JsonValue) -> Result<MetadataSchemaField, SchemaError> {
    let JsonValue::Object(object) = value else {
        return Err(SchemaError::Entry {
            index,
            message: "Input should be an object",
        });
    };
    for name in object.keys() {
        if !matches!(name.as_str(), "key" | "label") {
            return Err(SchemaError::Entry {
                index,
                message: "Extra inputs are not permitted",
            });
        }
    }
    let key = required_key(index, object.get("key"))?;
    Ok(MetadataSchemaField {
        key: key.to_owned(),
        label: optional_label(index, object.get("label"))?,
    })
}

fn required_key(index: usize, configured: Option<&JsonValue>) -> Result<&str, SchemaError> {
    let Some(value) = configured else {
        return Err(SchemaError::Field {
            index,
            field: "key",
            message: "Field required",
        });
    };
    let JsonValue::String(key) = value else {
        return Err(SchemaError::Field {
            index,
            field: "key",
            message: "Input should be a valid string",
        });
    };
    if key.is_empty() {
        return Err(SchemaError::Field {
            index,
            field: "key",
            message: "String should have at least 1 character",
        });
    }
    Ok(key)
}

fn optional_label(
    index: usize,
    configured: Option<&JsonValue>,
) -> Result<Option<String>, SchemaError> {
    match configured {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_) | JsonValue::Array(_),
        ) => Err(SchemaError::Field {
            index,
            field: "label",
            message: "Input should be a valid string",
        }),
    }
}

fn duplicate_keys(fields: &[MetadataSchemaField]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for field in fields {
        if !seen.insert(field.key.as_str()) {
            duplicates.insert(field.key.as_str());
        }
    }
    duplicates.into_iter().map(str::to_owned).collect()
}
