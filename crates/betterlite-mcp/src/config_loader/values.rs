use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth, MCPTransport};
use betterlite_models::decisions::Json;

use crate::server_payload::quote;

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

pub(crate) fn truthy(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null) => false,
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => number.as_f64().is_some_and(|parsed| parsed != 0.0_f64),
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(entries)) => !entries.is_empty(),
    }
}

pub(crate) fn python_repr(value: &Json) -> String {
    let mut rendered = String::new();
    push_repr(value, &mut rendered);
    rendered
}

fn push_repr(value: &Json, rendered: &mut String) {
    match value {
        Json::Null => rendered.push_str("None"),
        Json::Bool(true) => rendered.push_str("True"),
        Json::Bool(false) => rendered.push_str("False"),
        Json::Number(number) => rendered.push_str(&number.to_string()),
        Json::Text(text) => rendered.push_str(&quote(text)),
        Json::List(items) => {
            rendered.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    rendered.push_str(", ");
                }
                push_repr(item, rendered);
            }
            rendered.push(']');
        }
        Json::Object(entries) => {
            rendered.push('{');
            for (index, (key, entry)) in entries.iter().enumerate() {
                if index > 0 {
                    rendered.push_str(", ");
                }
                rendered.push_str(&quote(key));
                rendered.push_str(": ");
                push_repr(entry, rendered);
            }
            rendered.push('}');
        }
    }
}

pub(crate) fn optional_text_repr(value: Option<&str>) -> String {
    match value {
        None => "None".to_owned(),
        Some(text) => quote(text),
    }
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

pub(crate) const fn transport_seed(transport: Option<MCPTransport>) -> &'static str {
    match transport {
        None => "MCPTransport.http",
        Some(MCPTransport::Sse) => "sse",
        Some(MCPTransport::Http) => "http",
        Some(MCPTransport::Stdio) => "stdio",
    }
}
