use betterlite_models::decisions::Json;
use indexmap::IndexMap;

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
pub fn redacted_registry_dump(servers: &IndexMap<String, MCPServer>) -> IndexMap<String, Json> {
    let mut dump = IndexMap::new();
    for (server_id, server) in servers {
        let encoded = serde_json::to_string(server).unwrap_or_default();
        let Ok(Json::Object(mut fields)) = serde_json::from_str(&encoded) else {
            continue;
        };
        for (field, value) in &mut fields {
            if SECRET_FIELDS.contains(&field.as_str()) && !matches!(*value, Json::Null) {
                *value = Json::Text(REDACTED_VALUE.to_owned());
            }
        }
        dump.insert(server_id.clone(), Json::Object(fields));
    }
    dump
}
