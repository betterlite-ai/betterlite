use std::collections::BTreeMap;

use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::secret_map::{SecretMap, encrypt_secret_map};
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use secrecy::SecretString;
use serde::Serialize;

use super::McpEncodingError;
use super::credentials::{TOKEN_EXCHANGE_COLUMN_FIELDS, encrypt_credentials};
use super::dump::Dumper;
use super::env_vars::encrypt_global_env_var_values;
use super::request::{FieldsSet, McpServerRequest, PrepareMode, PreparedServerData};
use crate::mcp_stored_contracts::JsonValue;

const JSON_MAP_FIELDS: [&str; 2] = ["tool_name_to_display_name", "tool_name_to_description"];

pub fn prepare_mcp_server_data<N: NonceSource, D: SecretDiagnostics>(
    request: &McpServerRequest,
    mode: PrepareMode<'_>,
    cipher: &StoredCipher<N, D>,
) -> Result<PreparedServerData, McpEncodingError> {
    let mut data = Dumper { request, mode }.dump()?;
    match mode {
        PrepareMode::Create => {
            if !data.contains_key("alias") {
                data.insert("alias".to_owned(), JsonValue::Null);
            }
        }
        PrepareMode::Update {
            original_fields, ..
        } => {
            apply_partial_defaults(&mut data, original_fields);
        }
    }
    prepare_credentials(&mut data, mode, cipher)?;
    prepare_secret_map(&mut data, "static_headers", cipher)?;
    prepare_env_vars(&mut data, cipher)?;
    prepare_json_text(&mut data, "mcp_info")?;
    prepare_secret_map(&mut data, "env", cipher)?;
    prepare_json_text(&mut data, "tool_name_to_display_name")?;
    prepare_json_text(&mut data, "tool_name_to_description")?;
    if matches!(mode, PrepareMode::Create) {
        data.insert("is_byok".to_owned(), JsonValue::Bool(request.is_byok.0));
    }
    Ok(data)
}

fn apply_partial_defaults(data: &mut PreparedServerData, original_fields: &FieldsSet) {
    if matches!(data.get("alias"), Some(JsonValue::Null)) && !original_fields.contains("alias") {
        data.retain(|field, _value| field != "alias");
    }
    if matches!(data.get("allowed_tools"), Some(JsonValue::Null)) {
        data.insert("allowed_tools".to_owned(), JsonValue::Array(Vec::new()));
    }
    for field in JSON_MAP_FIELDS {
        if matches!(data.get(field), Some(JsonValue::Null)) {
            data.insert(field.to_owned(), JsonValue::Object(BTreeMap::new()));
        }
    }
}

fn prepare_credentials<N: NonceSource, D: SecretDiagnostics>(
    data: &mut PreparedServerData,
    mode: PrepareMode<'_>,
    cipher: &StoredCipher<N, D>,
) -> Result<(), McpEncodingError> {
    let Some(mut blob) = take_field(data, "credentials", as_object) else {
        return Ok(());
    };
    for field in TOKEN_EXCHANGE_COLUMN_FIELDS {
        let Some(lifted) = blob.remove(field) else {
            continue;
        };
        if !matches!(&lifted, JsonValue::Null) && !data.contains_key(field) {
            data.insert(field.to_owned(), lifted);
        }
    }
    if matches!(mode, PrepareMode::Create) {
        blob.retain(|_field, entry| !matches!(entry, JsonValue::Null));
    }
    encrypt_credentials(&mut blob, cipher)?;
    store_serialized(data, "credentials", &JsonValue::Object(blob))
}

fn prepare_secret_map<N: NonceSource, D: SecretDiagnostics>(
    data: &mut PreparedServerData,
    field: &'static str,
    cipher: &StoredCipher<N, D>,
) -> Result<(), McpEncodingError> {
    let Some(fields) = take_field(data, field, as_object) else {
        return Ok(());
    };
    let mut map = SecretMap::new();
    for (name, entry) in fields {
        let JsonValue::String(text) = entry else {
            return Err(McpEncodingError::SecretMapValue { field });
        };
        map.insert(name, SecretString::from(text));
    }
    let encoded = encrypt_secret_map(cipher, &map, None)?;
    data.insert(field.to_owned(), JsonValue::String(encoded));
    Ok(())
}

fn prepare_env_vars<N: NonceSource, D: SecretDiagnostics>(
    data: &mut PreparedServerData,
    cipher: &StoredCipher<N, D>,
) -> Result<(), McpEncodingError> {
    let Some(mut entries) = take_field(data, "env_vars", as_array) else {
        return Ok(());
    };
    encrypt_global_env_var_values(&mut entries, cipher)?;
    store_serialized(data, "env_vars", &entries)
}

fn prepare_json_text(
    data: &mut PreparedServerData,
    field: &'static str,
) -> Result<(), McpEncodingError> {
    let Some(value) = data.remove(field) else {
        return Ok(());
    };
    if matches!(&value, JsonValue::Null) {
        data.insert(field.to_owned(), value);
        return Ok(());
    }
    store_serialized(data, field, &value)
}

fn take_field<T, F>(data: &mut PreparedServerData, field: &'static str, shape: F) -> Option<T>
where
    F: FnOnce(JsonValue) -> Result<T, JsonValue>,
{
    let value = data.remove(field)?;
    match shape(value) {
        Ok(inner) => Some(inner),
        Err(other) => {
            data.insert(field.to_owned(), other);
            None
        }
    }
}

fn as_object(value: JsonValue) -> Result<BTreeMap<String, JsonValue>, JsonValue> {
    match value {
        JsonValue::Object(fields) => Ok(fields),
        other => Err(other),
    }
}

fn as_array(value: JsonValue) -> Result<Vec<JsonValue>, JsonValue> {
    match value {
        JsonValue::Array(items) => Ok(items),
        other => Err(other),
    }
}

fn store_serialized<T: Serialize>(
    data: &mut PreparedServerData,
    field: &'static str,
    value: &T,
) -> Result<(), McpEncodingError> {
    let text = serde_json::to_string(value).map_err(McpEncodingError::Json)?;
    data.insert(field.to_owned(), JsonValue::String(text));
    Ok(())
}
