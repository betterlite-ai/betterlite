use betterlite_core::api::response_chat_result::records::{ChatRole, ModelResponse};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// The chat fields the oracle compares: the id B919 writes, the message text and the finish reason
pub(super) fn response_view(response: &ModelResponse) -> Json {
    let choice = response.choices.first();
    let (role, content, finish_reason) = choice.map_or_else(
        || (Json::Null, Json::Null, Json::Null),
        |chosen| {
            (
                Json::Text(String::from(role_name(chosen.message.role))),
                optional_text(chosen.message.content.as_deref()),
                Json::Text(chosen.finish_reason.clone()),
            )
        },
    );
    let model = response
        .model
        .as_ref()
        .map_or_else(|| Json::Null, |name| Json::Text(name.clone()));
    Json::Object(IndexMap::from([
        (String::from("id"), Json::Text(response.id.clone())),
        (String::from("model"), model),
        (String::from("object"), Json::Text(response.object.clone())),
        (String::from("role"), role),
        (String::from("content"), content),
        (String::from("finish_reason"), finish_reason),
    ]))
}

// One B005 wrapper chunk, reduced to the fields a stream case compares
#[derive(Default)]
pub(super) struct StreamViews {
    chunks: Vec<Json>,
    ids: Vec<String>,
}

impl StreamViews {
    pub(super) fn push(&mut self, chunk: &Json) {
        let Json::Object(fields) = chunk else {
            return;
        };
        let Some(choice) = choice_of(fields) else {
            return;
        };
        let content = content_of(choice).filter(|text| !text.is_empty());
        let finish_reason = finish_of(choice);
        if content.is_none() && finish_reason.is_none() {
            return;
        }
        if content.is_some()
            && let Some(identifier) = text_of(fields.get("id"))
        {
            self.ids.push(identifier);
        }
        self.chunks.push(Json::Object(IndexMap::from([
            (String::from("content"), optional_text(content.as_deref())),
            (
                String::from("finish_reason"),
                optional_text(finish_reason.as_deref()),
            ),
        ])));
    }

    pub(super) fn finish(self) -> Json {
        let first = self.ids.first();
        let stable = first.is_some() && self.ids.iter().all(|identifier| Some(identifier) == first);
        Json::Object(IndexMap::from([
            (String::from("chunks"), Json::List(self.chunks)),
            (String::from("stable_id"), Json::Bool(stable)),
        ]))
    }
}

fn choice_of(fields: &IndexMap<String, Json>) -> Option<&IndexMap<String, Json>> {
    match fields.get("choices") {
        Some(Json::List(items)) => match items.first() {
            Some(Json::Object(choice)) => Some(choice),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => None,
        },
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => None,
    }
}

fn content_of(choice: &IndexMap<String, Json>) -> Option<String> {
    match choice.get("delta") {
        Some(Json::Object(delta)) => text_of(delta.get("content")),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

fn finish_of(choice: &IndexMap<String, Json>) -> Option<String> {
    text_of(choice.get("finish_reason"))
}

fn text_of(value: Option<&Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn optional_text(text: Option<&str>) -> Json {
    text.map_or_else(|| Json::Null, |value| Json::Text(value.to_owned()))
}

const fn role_name(role: ChatRole) -> &'static str {
    match role {
        ChatRole::Assistant => "assistant",
        ChatRole::User => "user",
        ChatRole::System => "system",
        ChatRole::Tool => "tool",
        ChatRole::Function => "function",
    }
}
