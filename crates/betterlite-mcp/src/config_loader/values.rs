use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub(crate) fn stored_json(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(stored_json).collect()),
        Json::Object(entries) => JsonValue::Object(
            entries
                .iter()
                .map(|(key, entry)| (key.clone(), stored_json(entry)))
                .collect(),
        ),
    }
}

pub(crate) fn stored_entries(entries: &IndexMap<String, Json>) -> BTreeMap<String, JsonValue> {
    entries
        .iter()
        .map(|(key, entry)| (key.clone(), stored_json(entry)))
        .collect()
}

pub(crate) const fn auth_text(auth: MCPAuth) -> &'static str {
    match auth {
        MCPAuth::None => "none",
        MCPAuth::ApiKey => "api_key",
        MCPAuth::BearerToken => "bearer_token",
        MCPAuth::Basic => "basic",
        MCPAuth::Authorization => "authorization",
        MCPAuth::Oauth2 => "oauth2",
        MCPAuth::AwsSigv4 => "aws_sigv4",
        MCPAuth::Token => "token",
        MCPAuth::Oauth2TokenExchange => "oauth2_token_exchange",
        MCPAuth::Oauth2IdJag => "oauth2_id_jag",
        MCPAuth::TruePassthrough => "true_passthrough",
        MCPAuth::OauthDelegate => "oauth_delegate",
    }
}
