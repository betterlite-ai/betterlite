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
        authorization_servers: servers(fields.get("authorization_servers"))?,
        scopes_supported: scopes_value(fields.get("scopes_supported"), "scopes_supported")?,
        scopes: scopes_value(fields.get("scopes"), "scopes")?,
        resource: fields.get("resource").cloned(),
        issuer: fields.get("issuer").cloned(),
        authorization_endpoint: text_field(
            fields.get("authorization_endpoint"),
            "authorization_endpoint",
        )?,
        token_endpoint: text_field(fields.get("token_endpoint"), "token_endpoint")?,
        registration_endpoint: text_field(
            fields.get("registration_endpoint"),
            "registration_endpoint",
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

fn servers(value: Option<&JsonValue>) -> Result<Option<Vec<JsonValue>>, DocumentError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Array(entries)) => Ok(Some(entries.clone())),
        Some(
            JsonValue::String(_) | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_),
        ) => Err(DocumentError::Field {
            field: "authorization_servers",
        }),
    }
}

fn scopes_value(
    value: Option<&JsonValue>,
    field: &'static str,
) -> Result<Option<ScopesValue>, DocumentError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(ScopesValue::Text(text.clone()))),
        Some(JsonValue::Array(entries)) => Ok(Some(ScopesValue::List(entries.clone()))),
        Some(JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_)) => {
            Err(DocumentError::Field { field })
        }
    }
}

fn text_field(
    value: Option<&JsonValue>,
    field: &'static str,
) -> Result<Option<String>, DocumentError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(DocumentError::Field { field }),
    }
}
