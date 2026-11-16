pub(super) mod cipher;
pub(super) mod database;
pub(super) mod fake;
pub(super) mod logs;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp_server_encoding::request::{FieldsSet, McpServerRequest};

pub(super) fn request(text: &str) -> McpServerRequest {
    serde_json::from_str(text).expect("the case payload is a valid MCP server request")
}

// The endpoint's fields_set is the set of keys the caller sent, which is what a partial update writes.
pub(super) fn update_payload(text: &str) -> (McpServerRequest, FieldsSet) {
    let keys: BTreeMap<String, JsonData> =
        serde_json::from_str(text).expect("the payload is a JSON object");
    let fields = keys.into_keys().collect::<BTreeSet<String>>();
    (request(text), fields)
}

pub(super) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the expected value is JSON")
}
