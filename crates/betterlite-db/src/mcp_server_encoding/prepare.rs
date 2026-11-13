use std::collections::BTreeMap;

use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::secret_map::{SecretMap, encrypt_secret_map};
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use secrecy::SecretString;

use super::McpEncodingError;
use super::credentials::{TOKEN_EXCHANGE_COLUMN_FIELDS, encrypt_credentials};
use super::dump::Dumper;
use super::env_vars::encrypt_global_env_var_values;
use super::request::{FieldsSet, McpServerRequest, PrepareMode, PreparedServerData};
use crate::mcp_stored_contracts::JsonValue;

const JSON_MAP_FIELDS: [&str; 2] = ["tool_name_to_display_name", "tool_name_to_description"];

pub fn prepare_mcp_server_data<N, D>(
    request: &McpServerRequest,
    mode: PrepareMode<'_>,
    cipher: &StoredCipher<N, D>,
) -> Result<PreparedServerData, McpEncodingError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
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

fn prepare_credentials<N, D>(
    data: &mut PreparedServerData,
    mode: PrepareMode<'_>,
    cipher: &StoredCipher<N, D>,
) -> Result<(), McpEncodingError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Some(value) = data.remove("credentials") else {
        return Ok(());
    };
    let mut blob = match value {
        JsonValue::Object(fields) => fields,
        other => {
            data.insert("credentials".to_owned(), other);
            return Ok(());
        }
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
    let text = serde_json::to_string(&JsonValue::Object(blob)).map_err(McpEncodingError::Json)?;
    data.insert("credentials".to_owned(), JsonValue::String(text));
    Ok(())
}

fn prepare_secret_map<N, D>(
    data: &mut PreparedServerData,
    field: &'static str,
    cipher: &StoredCipher<N, D>,
) -> Result<(), McpEncodingError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Some(value) = data.remove(field) else {
        return Ok(());
    };
    let fields = match value {
        JsonValue::Object(fields) => fields,
        other => {
            data.insert(field.to_owned(), other);
            return Ok(());
        }
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

fn prepare_env_vars<N, D>(
    data: &mut PreparedServerData,
    cipher: &StoredCipher<N, D>,
) -> Result<(), McpEncodingError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Some(value) = data.remove("env_vars") else {
        return Ok(());
    };
    let mut entries = match value {
        JsonValue::Array(entries) => entries,
        other => {
            data.insert("env_vars".to_owned(), other);
            return Ok(());
        }
    };
    encrypt_global_env_var_values(&mut entries, cipher)?;
    let text = serde_json::to_string(&entries).map_err(McpEncodingError::Json)?;
    data.insert("env_vars".to_owned(), JsonValue::String(text));
    Ok(())
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
    let text = serde_json::to_string(&value).map_err(McpEncodingError::Json)?;
    data.insert(field.to_owned(), JsonValue::String(text));
    Ok(())
}
