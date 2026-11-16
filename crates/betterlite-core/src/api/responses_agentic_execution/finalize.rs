use crate::api::responses_hook_policy::safety::{is_refused_tool_use_block, refused_tool_ids};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::value::RawValue;
pub(crate) fn finalize_refused_response(
    response: &RawValue,
    tool_calls: &Json,
) -> Result<Json, serde_json::Error> {
    let value: Json = serde_json::from_str(response.get())?;
    match value {
        Json::Object(fields) => Ok(finalize_object(fields, tool_calls)),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Ok(other)
        }
    }
}
fn finalize_object(mut fields: IndexMap<String, Json>, tool_calls: &Json) -> Json {
    let Some(Json::List(blocks)) = fields.get("content").cloned() else {
        return Json::Object(fields);
    };
    let refused = refused_tool_ids(tool_calls);
    let kept: Vec<Json> = blocks
        .iter()
        .filter(|block| !is_refused_tool_use_block(block, &refused.ids, &refused.names))
        .cloned()
        .collect();
    let client_tool_use_remains = kept.iter().any(|block| match block {
        Json::Object(item) => {
            matches!(item.get("type"), Some(Json::Text(kind)) if kind == "tool_use")
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    });
    let stop_reason_is_tool_use = matches!(
        fields.get("stop_reason"),
        Some(Json::Text(reason)) if reason == "tool_use"
    );
    if !client_tool_use_remains && stop_reason_is_tool_use {
        fields.insert("stop_reason".to_owned(), Json::Text("end_turn".to_owned()));
    }
    fields.insert("content".to_owned(), Json::List(kept));
    Json::Object(fields)
}
