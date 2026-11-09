use betterlite_db::mcp_stored_contracts::JsonValue;
use thiserror::Error;

use crate::server_oauth::merge::{ScopesInput, extract_scopes};

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DocumentError {
    #[error("the document is not valid JSON")]
    Json,
    #[error("the document root is not a JSON object")]
    NotObject,
    #[error("the {field} field has a type the document does not declare")]
    Field { field: &'static str },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScopesValue {
    Text(String),
    List(Vec<JsonValue>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetadataDocument {
    pub authorization_servers: Option<Vec<JsonValue>>,
    pub scopes_supported: Option<ScopesValue>,
    pub scopes: Option<ScopesValue>,
    pub resource: Option<JsonValue>,
    pub issuer: Option<JsonValue>,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub registration_endpoint: Option<String>,
}

pub fn decode_metadata_document(bytes: &[u8]) -> Result<MetadataDocument, DocumentError> {
    let value: JsonValue = serde_json::from_slice(bytes).map_err(|_error| DocumentError::Json)?;
    let JsonValue::Object(fields) = value else {
        return Err(DocumentError::NotObject);
    };
    Ok(MetadataDocument {
        authorization_servers: field(
            fields.get("authorization_servers"),
            "authorization_servers",
            entries,
        )?,
        scopes_supported: field(
            fields.get("scopes_supported"),
            "scopes_supported",
            scopes_entries,
        )?,
        scopes: field(fields.get("scopes"), "scopes", scopes_entries)?,
        resource: fields.get("resource").cloned(),
        issuer: fields.get("issuer").cloned(),
        authorization_endpoint: field(
            fields.get("authorization_endpoint"),
            "authorization_endpoint",
            text_entry,
        )?,
        token_endpoint: field(fields.get("token_endpoint"), "token_endpoint", text_entry)?,
        registration_endpoint: field(
            fields.get("registration_endpoint"),
            "registration_endpoint",
            text_entry,
        )?,
    })
}

#[must_use]
pub fn document_scopes(value: Option<&ScopesValue>) -> Option<Vec<String>> {
    match value {
        Some(ScopesValue::Text(text)) => extract_scopes(ScopesInput::Text(text)),
        Some(ScopesValue::List(entries)) => extract_scopes(ScopesInput::List(entries)),
        None => None,
    }
}

#[must_use]
pub fn document_text(value: Option<&JsonValue>) -> Option<&str> {
    match value {
        Some(JsonValue::String(text)) => Some(text),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}

fn field<T, C>(
    value: Option<&JsonValue>,
    name: &'static str,
    convert: C,
) -> Result<Option<T>, DocumentError>
where
    C: FnOnce(&JsonValue) -> Option<T>,
{
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(inner) => convert(inner)
            .map(Some)
            .ok_or(DocumentError::Field { field: name }),
    }
}

fn entries(value: &JsonValue) -> Option<Vec<JsonValue>> {
    if let JsonValue::Array(values) = value {
        return Some(values.clone());
    }
    None
}

fn scopes_entries(value: &JsonValue) -> Option<ScopesValue> {
    match value {
        JsonValue::String(text) => Some(ScopesValue::Text(text.clone())),
        JsonValue::Array(values) => Some(ScopesValue::List(values.clone())),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_) => None,
    }
}

fn text_entry(value: &JsonValue) -> Option<String> {
    if let JsonValue::String(text) = value {
        return Some(text.clone());
    }
    None
}
