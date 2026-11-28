use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::VertexPassthroughError;

pub(super) fn set_body_id(body: &mut Json, call_id: &str) -> Result<(), VertexPassthroughError> {
    let Json::Object(fields) = body else {
        return Err(VertexPassthroughError::Shape {
            kind: "chat response",
        });
    };
    fields.insert("id".to_owned(), Json::Text(call_id.to_owned()));
    Ok(())
}

// litellm.ModelResponse() with no body: its one default choice and the fields the constructor always sets.
pub(super) fn empty_chat_response(created: i64) -> Json {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("assistant".to_owned()));
    message.insert("content".to_owned(), Json::Null);
    message.insert("tool_calls".to_owned(), Json::Null);
    message.insert("function_call".to_owned(), Json::Null);
    let mut choice = IndexMap::new();
    choice.insert("finish_reason".to_owned(), Json::Text("stop".to_owned()));
    choice.insert("index".to_owned(), Json::Number(Number::from(0_i64)));
    choice.insert("message".to_owned(), Json::Object(message));
    let mut fields = IndexMap::new();
    fields.insert("id".to_owned(), Json::Text(String::new()));
    fields.insert(
        "object".to_owned(),
        Json::Text("chat.completion".to_owned()),
    );
    fields.insert("created".to_owned(), Json::Number(Number::from(created)));
    fields.insert("model".to_owned(), Json::Null);
    fields.insert("system_fingerprint".to_owned(), Json::Null);
    fields.insert("choices".to_owned(), Json::List(vec![Json::Object(choice)]));
    Json::Object(fields)
}
