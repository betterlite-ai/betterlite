use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::values::python_repr;
use crate::server_record::MCPServer;

pub const REDACTED_VALUE: &str = "**REDACTED**";

const SECRET_FIELDS: [&str; 5] = [
    "authentication_token",
    "client_secret",
    "client_private_key",
    "aws_secret_access_key",
    "aws_session_token",
];

#[must_use]
pub fn redacted_registry_dump(
    servers: &IndexMap<String, MCPServer>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut dump = BTreeMap::new();
    for (server_id, server) in servers {
        let encoded = serde_json::to_string(server).unwrap_or_default();
        let fields: BTreeMap<String, Json> = serde_json::from_str(&encoded).unwrap_or_default();
        dump.insert(server_id.clone(), redacted_fields(&fields));
    }
    dump
}

fn redacted_fields(fields: &BTreeMap<String, Json>) -> BTreeMap<String, String> {
    let mut redacted = BTreeMap::new();
    for (field, value) in fields {
        let masked = SECRET_FIELDS.contains(&field.as_str()) && !matches!(value, Json::Null);
        let text = if masked {
            REDACTED_VALUE.to_owned()
        } else {
            dump_text(value)
        };
        redacted.insert(field.clone(), text);
    }
    redacted
}

fn dump_text(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => python_repr(value),
    }
}
