use betterlite_db::mcp_stored_contracts::JsonValue;

use crate::server_record::{MCPOAuthMetadata, MCPServer};

#[must_use]
pub fn merge_discovered(server: &MCPServer, metadata: Option<&MCPOAuthMetadata>) -> MCPServer {
    let Some(document) = metadata else {
        return server.clone();
    };
    let mut resolved = server.clone();
    if server.fields.scopes.as_ref().is_none_or(Vec::is_empty) {
        resolved.fields.scopes.clone_from(&document.scopes);
    }
    let discovered_issuer = if document.from_origin_fallback {
        None
    } else {
        document.discovered_issuer.clone()
    };
    if server.fields.issuer.as_deref().is_none_or(str::is_empty) {
        resolved.fields.issuer = discovered_issuer;
    }
    fill_missing(
        &mut resolved.fields.authorization_url,
        document.authorization_url.as_deref(),
    );
    fill_missing(
        &mut resolved.fields.token_url,
        document.token_url.as_deref(),
    );
    fill_missing(
        &mut resolved.fields.registration_url,
        document.registration_url.as_deref(),
    );
    resolved
}

fn fill_missing(target: &mut Option<String>, discovered: Option<&str>) {
    if target.as_deref().is_none_or(str::is_empty) {
        *target = discovered.map(str::to_owned);
    }
}

#[derive(Clone, Copy)]
pub enum ScopesInput<'text> {
    Text(&'text str),
    List(&'text [JsonValue]),
    Missing,
}

#[must_use]
pub fn extract_scopes(input: ScopesInput<'_>) -> Option<Vec<String>> {
    match input {
        ScopesInput::Text(text) => {
            let scopes: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
            (!scopes.is_empty()).then_some(scopes)
        }
        ScopesInput::List(values) => {
            let scopes: Vec<String> = values
                .iter()
                .filter_map(|value| match value {
                    JsonValue::String(text) if !text.trim().is_empty() => Some(text.clone()),
                    JsonValue::String(_)
                    | JsonValue::Null
                    | JsonValue::Bool(_)
                    | JsonValue::Number(_)
                    | JsonValue::Array(_)
                    | JsonValue::Object(_) => None,
                })
                .collect();
            (!scopes.is_empty()).then_some(scopes)
        }
        ScopesInput::Missing => None,
    }
}
