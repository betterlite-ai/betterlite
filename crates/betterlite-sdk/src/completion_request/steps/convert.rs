use super::super::args::JsonObject;
use super::super::error::{CompletionRequestError, RequestResult};
use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_prompts::builtin::Message as BuiltinMessage;
use betterlite_prompts::message_content::Message;
use serde::Serialize;
use serde::de::DeserializeOwned;
pub(crate) fn object_from_params(params: &Params) -> RequestResult<JsonObject> {
    let value = serde_json::to_value(params)?;
    Ok(serde_json::from_value(value)?)
}
pub(crate) fn params_from_object(object: &JsonObject) -> RequestResult<Params> {
    let value = serde_json::to_value(object)?;
    Ok(serde_json::from_value(value)?)
}
pub(crate) fn messages_from_json(messages: &[Json]) -> RequestResult<Vec<Message>> {
    let value = serde_json::to_value(messages)?;
    serde_json::from_value(value).map_err(|source| CompletionRequestError::Field {
        field: "messages",
        source,
    })
}
pub(crate) fn json_from_messages(messages: &[Message]) -> RequestResult<Vec<Json>> {
    let value = serde_json::to_value(messages)?;
    Ok(serde_json::from_value(value)?)
}
pub(crate) fn builtin_messages(messages: &[Message]) -> RequestResult<Vec<BuiltinMessage>> {
    let value = serde_json::to_value(messages)?;
    let mut objects: Vec<JsonObject> = serde_json::from_value(value)?;
    for object in &mut objects {
        object.entry(String::from("content")).or_insert(Json::Null);
    }
    let filled = serde_json::to_value(&objects)?;
    serde_json::from_value(filled).map_err(|source| CompletionRequestError::Field {
        field: "messages",
        source,
    })
}
pub(crate) fn messages_from_builtin(messages: &[BuiltinMessage]) -> RequestResult<Vec<Message>> {
    let value = serde_json::to_value(messages)?;
    let mut objects: Vec<JsonObject> = serde_json::from_value(value)?;
    for object in &mut objects {
        if matches!(object.get("content"), Some(Json::Null)) {
            object.shift_remove("content");
        }
    }
    let stripped = serde_json::to_value(&objects)?;
    serde_json::from_value(stripped).map_err(|source| CompletionRequestError::Field {
        field: "messages",
        source,
    })
}
pub(crate) fn json_value(value: &Json) -> RequestResult<JsonValue> {
    let encoded = serde_json::to_value(value)?;
    Ok(serde_json::from_value(encoded)?)
}
pub(crate) fn typed_from<T>(value: &Json, field: &'static str) -> RequestResult<T>
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_value(value)?;
    serde_json::from_value(encoded)
        .map_err(|source| CompletionRequestError::Field { field, source })
}
pub(crate) fn json_of<T>(value: &T) -> RequestResult<Json>
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value)?;
    Ok(serde_json::from_value(encoded)?)
}
pub(crate) fn json_from_value(value: &JsonValue) -> RequestResult<Json> {
    let encoded = serde_json::to_value(value)?;
    Ok(serde_json::from_value(encoded)?)
}
