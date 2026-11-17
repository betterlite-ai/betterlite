use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use uuid::Uuid;

// LiteLLM logs a ModelResponse built from the model and usage alone: the reply the provider never sent.
pub(super) fn response_envelope(model_name: &str, usage: Json, created: i64) -> Json {
    Json::Object(IndexMap::from([
        (
            "id".to_owned(),
            Json::Text(format!("chatcmpl-{}", Uuid::new_v4())),
        ),
        ("created".to_owned(), Json::Number(Number::from(created))),
        ("model".to_owned(), Json::Text(model_name.to_owned())),
        (
            "object".to_owned(),
            Json::Text("chat.completion".to_owned()),
        ),
        ("system_fingerprint".to_owned(), Json::Null),
        ("choices".to_owned(), Json::List(vec![completion_choice()])),
        ("usage".to_owned(), usage),
    ]))
}

// ModelResponse fills a missing choices list with one default choice, an empty assistant message.
fn completion_choice() -> Json {
    Json::Object(IndexMap::from([
        ("finish_reason".to_owned(), Json::Text("stop".to_owned())),
        ("index".to_owned(), Json::Number(Number::from(0_i32))),
        ("message".to_owned(), completion_message()),
    ]))
}

fn completion_message() -> Json {
    Json::Object(IndexMap::from([
        ("content".to_owned(), Json::Null),
        ("role".to_owned(), Json::Text("assistant".to_owned())),
        ("tool_calls".to_owned(), Json::Null),
        ("function_call".to_owned(), Json::Null),
    ]))
}
