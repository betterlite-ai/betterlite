use super::super::args::JsonObject;
use super::super::error::{CompletionRequestError, RequestResult};
use betterlite_models::decisions::Json;
use betterlite_prompts::builtin::Message as BuiltinMessage;
use betterlite_prompts::message_content::Message;
use serde::Serialize;
use serde::de::DeserializeOwned;
// The workspace keeps several JSON types apart; a round trip through serde is the one conversion.
pub(crate) fn convert<T, S>(value: &S) -> RequestResult<T>
where
    T: DeserializeOwned,
    S: Serialize + ?Sized,
{
    match serde_json::to_value(value) {
        Ok(encoded) => Ok(serde_json::from_value(encoded)?),
        Err(source) => Err(CompletionRequestError::Json { source }),
    }
}
pub(crate) fn typed_from<T>(value: &Json, field: &'static str) -> RequestResult<T>
where
    T: DeserializeOwned,
{
    match serde_json::to_value(value) {
        Ok(encoded) => serde_json::from_value(encoded)
            .map_err(|source| CompletionRequestError::Field { field, source }),
        Err(source) => Err(CompletionRequestError::Json { source }),
    }
}
// The builtin prompt helpers take a message whose content is always present; LiteLLM's dicts may omit it.
pub(crate) fn builtin_messages(messages: &[Message]) -> RequestResult<Vec<BuiltinMessage>> {
    let mut objects: Vec<JsonObject> = convert(messages)?;
    for object in &mut objects {
        object.entry(String::from("content")).or_insert(Json::Null);
    }
    convert(&objects)
}
pub(crate) fn messages_from_builtin(messages: &[BuiltinMessage]) -> RequestResult<Vec<Message>> {
    let mut objects: Vec<JsonObject> = convert(messages)?;
    for object in &mut objects {
        if matches!(object.get("content"), Some(Json::Null)) {
            object.shift_remove("content");
        }
    }
    convert(&objects)
}
