use super::support::safe_structure;
use crate::python_json;
use crate::spend_sanitize::value::{SpendMapping, SpendValue, to_json};

const REALTIME_CALL_TYPE: &str = "_arealtime";

#[must_use]
pub fn storage_messages(payload: Option<&SpendMapping>, store_bodies: bool) -> String {
    if !store_bodies {
        return "{}".to_owned();
    }
    let Some(stored_payload) = payload else {
        return "{}".to_owned();
    };
    if !matches!(
        stored_payload.entries.get("call_type"),
        Some(SpendValue::Text(name)) if name == REALTIME_CALL_TYPE
    ) {
        return "{}".to_owned();
    }
    let Some(messages) = stored_payload.entries.get("messages") else {
        return "{}".to_owned();
    };
    if matches!(messages, SpendValue::Null) {
        return "{}".to_owned();
    }
    python_json::dumps(&safe_structure(&to_json(messages), 0))
}
