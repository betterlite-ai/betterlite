use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::Number;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct TextCompletionChoice {
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub text: String,
}

// The legacy completion answer as LiteLLM builds it. `usage` is the usage record dumped with unset fields left out.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct TextCompletionResponse {
    pub id: String,
    pub created: i64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<TextCompletionChoice>,
    #[serde(default)]
    pub usage: Option<Object>,
}

// Straiker has no reader for a `text_completion` answer on a gateway, so a completion is presented as one user turn and one assistant turn in the chat shape it scores.
#[must_use]
pub fn text_completion_as_chat(response: &TextCompletionResponse) -> Object {
    let choices = response
        .choices
        .iter()
        .enumerate()
        .map(|(index, choice)| Json::Object(chat_choice(index, choice)))
        .collect();
    let mut chat = Object::from([
        ("id".to_owned(), Json::Text(response.id.clone())),
        (
            "object".to_owned(),
            Json::Text("chat.completion".to_owned()),
        ),
        (
            "created".to_owned(),
            Json::Number(Number::from(response.created)),
        ),
        (
            "model".to_owned(),
            response.model.clone().map_or(Json::Null, Json::Text),
        ),
        ("choices".to_owned(), Json::List(choices)),
    ]);
    if let Some(usage) = response.usage.as_ref().filter(|usage| !usage.is_empty()) {
        chat.insert("usage".to_owned(), Json::Object(usage.clone()));
    }
    chat
}

fn chat_choice(index: usize, choice: &TextCompletionChoice) -> Object {
    let message = Object::from([
        ("role".to_owned(), Json::Text("assistant".to_owned())),
        ("content".to_owned(), Json::Text(choice.text.clone())),
    ]);
    Object::from([
        ("index".to_owned(), Json::Number(Number::from(index))),
        (
            "finish_reason".to_owned(),
            choice.finish_reason.clone().map_or(Json::Null, Json::Text),
        ),
        ("message".to_owned(), Json::Object(message)),
    ])
}

mod tests;
