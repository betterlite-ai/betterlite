use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::text::optional_str;

pub const SPEECH_OUTPUT: &str = "speech-output";

// Python resolves the response family with isinstance; the logging pipeline names it here instead
#[derive(Clone, Copy, Debug)]
pub enum ResponseObj<'response> {
    Chat(&'response Json),
    Embedding,
    Image(&'response Json),
    List(&'response Json),
    Dict(&'response Json),
    Rerank(&'response Json),
    ResponsesApi(&'response Json),
    Speech,
    TextCompletion(&'response Json),
    Transcription(&'response Json),
}

// litellm.integrations.langfuse.LangFuseLogger._get_chat_content_for_langfuse; the message model dump is carried structured instead of as pydantic's JSON text
#[must_use]
pub fn chat_content(response: &Json) -> Option<Json> {
    match first_choice(response)?.get("message") {
        Some(Json::Null) | None => None,
        Some(message) => Some(message.clone()),
    }
}

// litellm.integrations.langfuse.LangFuseLogger._get_text_completion_content_for_langfuse
#[must_use]
pub fn text_completion_content(response: &Json) -> Option<Json> {
    let choice = first_choice(response)?;
    let text = optional_str(choice.get("text"))?;
    Some(Json::Text(text))
}

// litellm.integrations.langfuse.LangFuseLogger._get_responses_api_content_for_langfuse
#[must_use]
pub fn responses_api_content(response: &Json) -> Option<Json> {
    match present_field(response, "output") {
        Some(Json::List(items)) => (!items.is_empty()).then_some(Json::List(items)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => None,
    }
}

// response.get(name, None) for a JSON object, with an explicit null read as absent
#[must_use]
pub fn present_field(response: &Json, name: &str) -> Option<Json> {
    match response {
        Json::Object(fields) => fields
            .get(name)
            .filter(|value| !matches!(value, Json::Null))
            .cloned(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn first_choice(response: &Json) -> Option<&IndexMap<String, Json>> {
    let Json::Object(fields) = response else {
        return None;
    };
    let choices = match fields.get("choices") {
        Some(Json::List(choices)) => choices,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => {
            return None;
        }
    };
    match choices.first() {
        Some(Json::Object(choice)) => Some(choice),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}
