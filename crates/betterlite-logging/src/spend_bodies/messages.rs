use super::support::{as_object, safe_structure};
use crate::python_json;
use betterlite_models::decisions::Json;
const REALTIME_CALL_TYPE: &str = "_arealtime";
#[must_use]
pub fn storage_messages(payload: Option<&Json>, store_bodies: bool) -> String {
    if !store_bodies {
        return "{}".to_owned();
    }
    let Some(fields) = payload.and_then(as_object) else {
        return "{}".to_owned();
    };
    if !matches!(
        fields.get("call_type"),
        Some(Json::Text(name)) if name == REALTIME_CALL_TYPE
    ) {
        return "{}".to_owned();
    }
    let Some(messages) = fields.get("messages") else {
        return "{}".to_owned();
    };
    if matches!(messages, Json::Null) {
        return "{}".to_owned();
    }
    python_json::dumps(&safe_structure(messages, 0))
}
