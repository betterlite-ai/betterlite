use super::support::{as_object, safe_structure};
use crate::python_json;
use betterlite_models::decisions::Json;
const REALTIME_CALL_TYPE: &str = "_arealtime";
#[must_use]
pub fn storage_messages(payload: Option<&Json>, store_bodies: bool) -> String {
    let Some(fields) = payload.filter(|_| store_bodies).and_then(as_object) else {
        return "{}".to_owned();
    };
    let Some(messages) = fields
        .get("messages")
        .filter(|messages| !matches!(messages, Json::Null))
    else {
        return "{}".to_owned();
    };
    if matches!(fields.get("call_type"), Some(Json::Text(name)) if name == REALTIME_CALL_TYPE) {
        return python_json::dumps(&safe_structure(messages, 0));
    }
    "{}".to_owned()
}
