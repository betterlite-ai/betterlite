use std::collections::BTreeMap;

use betterlite_mcp::request_headers::client_headers::{
    mcp_auth_header, oauth2_headers, server_auth_headers,
};
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::ResponsesParametersError;

#[derive(Debug, Clone)]
pub struct McpHeaders {
    pub auth_header: Option<SecretString>,
    pub server_auth_headers: Option<BTreeMap<String, BTreeMap<String, SecretString>>>,
    pub oauth2_headers: Option<BTreeMap<String, SecretString>>,
    pub raw_headers: Option<BTreeMap<String, String>>,
}

pub fn extract_mcp_headers(
    secret_fields: Option<&JsonValue>,
    tools: Option<&[JsonValue]>,
    configured_name: &str,
) -> Result<McpHeaders, ResponsesParametersError> {
    let raw = raw_headers_from_secret_fields(secret_fields)?;
    let mut headers = McpHeaders {
        auth_header: None,
        server_auth_headers: None,
        oauth2_headers: None,
        raw_headers: raw.clone(),
    };
    if let Some(pairs) = raw.as_ref().filter(|pairs| !pairs.is_empty()) {
        let parsed = raw_headers_from(pairs)?;
        headers.auth_header = mcp_auth_header(&parsed, configured_name);
        headers.server_auth_headers = Some(server_auth_headers(&parsed));
        headers.oauth2_headers = Some(oauth2_headers(&parsed));
    }
    if let Some(tool_list) = tools {
        for tool in tool_list {
            merge_tool_headers(&mut headers, tool)?;
        }
    }
    Ok(headers)
}

fn raw_headers_from_secret_fields(
    secret_fields: Option<&JsonValue>,
) -> Result<Option<BTreeMap<String, String>>, ResponsesParametersError> {
    let Some(present) = secret_fields.filter(|value| !matches!(value, JsonValue::Null)) else {
        return Ok(None);
    };
    let fields = object_fields(present, "secret_fields")?;
    match fields
        .get("raw_headers")
        .filter(|value| !matches!(value, JsonValue::Null))
    {
        None => Ok(None),
        Some(value) => text_map(object_fields(value, "raw_headers")?, "raw_headers").map(Some),
    }
}

const fn object_fields<'value>(
    value: &'value JsonValue,
    field: &'static str,
) -> Result<&'value IndexMap<String, JsonValue>, ResponsesParametersError> {
    match value {
        JsonValue::Object(fields) => Ok(fields),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Text(_)
        | JsonValue::Array(_) => Err(ResponsesParametersError::NotAnObject { field }),
    }
}

fn text_map(
    fields: &IndexMap<String, JsonValue>,
    field: &'static str,
) -> Result<BTreeMap<String, String>, ResponsesParametersError> {
    fields
        .iter()
        .map(|(name, value)| {
            let JsonValue::Text(text) = value else {
                return Err(ResponsesParametersError::NotText { field });
            };
            Ok((name.clone(), text.clone()))
        })
        .collect()
}

fn raw_headers_from(
    pairs: &BTreeMap<String, String>,
) -> Result<RawHeaders, ResponsesParametersError> {
    let bytes: Vec<(&[u8], &[u8])> = pairs
        .iter()
        .map(|(name, value)| (name.as_bytes(), value.as_bytes()))
        .collect();
    RawHeaders::from_pairs(&bytes)
        .map_err(|source| ResponsesParametersError::DuplicateAuthorization { source })
}

fn merge_tool_headers(
    headers: &mut McpHeaders,
    tool: &JsonValue,
) -> Result<(), ResponsesParametersError> {
    let JsonValue::Object(fields) = tool else {
        return Ok(());
    };
    if !matches!(fields.get("type"), Some(JsonValue::Text(kind)) if kind == "mcp") {
        return Ok(());
    }
    let Some(JsonValue::Object(tool_headers)) = fields.get("headers") else {
        return Ok(());
    };
    if tool_headers.is_empty() {
        return Ok(());
    }
    let pairs = text_map(tool_headers, "tools.headers")?;
    let parsed = raw_headers_from(&pairs)?;
    let from_tool = server_auth_headers(&parsed);
    if !from_tool.is_empty() {
        let target = headers.server_auth_headers.get_or_insert_default();
        for (alias, entries) in from_tool {
            target.entry(alias).or_default().extend(entries);
        }
    }
    headers.raw_headers.get_or_insert_default().extend(pairs);
    Ok(())
}
