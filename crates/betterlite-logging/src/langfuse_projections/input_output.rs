use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::content::{
    ResponseObj, SPEECH_OUTPUT, chat_content, present_field, responses_api_content,
    text_completion_content,
};

pub struct ContentRequest<'call> {
    pub call_type: Option<&'call str>,
    pub input: Option<&'call Json>,
    pub level: &'call str,
    pub status_message: Option<&'call str>,
}

// litellm.integrations.langfuse.LangFuseLogger._get_langfuse_input_output_content
#[must_use]
pub fn get_input_output_content(
    prompt: &IndexMap<String, Json>,
    response_obj: Option<&ResponseObj<'_>>,
    request: &ContentRequest<'_>,
) -> (Option<Json>, Option<Json>) {
    if request.level == "ERROR"
        && let Some(status_message) = request.status_message
    {
        let output = Json::Text(status_message.to_owned());
        return (Some(Json::Object(prompt.clone())), Some(output));
    }
    let Some(response) = response_obj else {
        return (None, None);
    };
    if request.call_type == Some("embedding") || matches!(response, ResponseObj::Embedding) {
        return (Some(Json::Object(prompt.clone())), None);
    }
    response_content(prompt, response, request)
}

fn response_content(
    prompt: &IndexMap<String, Json>,
    response: &ResponseObj<'_>,
    request: &ContentRequest<'_>,
) -> (Option<Json>, Option<Json>) {
    let prompt_value = || Json::Object(prompt.clone());
    match response {
        ResponseObj::Chat(body) => (Some(prompt_value()), chat_content(body)),
        ResponseObj::TextCompletion(body) => (Some(prompt_value()), text_completion_content(body)),
        ResponseObj::Speech => (
            Some(prompt_value()),
            Some(Json::Text(SPEECH_OUTPUT.to_owned())),
        ),
        ResponseObj::Image(body) => (Some(prompt_value()), present_field(body, "data")),
        ResponseObj::Transcription(body) => (Some(prompt_value()), present_field(body, "text")),
        ResponseObj::Rerank(body) => (Some(prompt_value()), present_field(body, "results")),
        ResponseObj::ResponsesApi(body) => (Some(prompt_value()), responses_api_content(body)),
        ResponseObj::List(items) if request.call_type == Some("_arealtime") => {
            let input = match request.input {
                Some(Json::Null) | None => None,
                Some(value) => Some(value.clone()),
            };
            (input, Some((*items).clone()))
        }
        ResponseObj::Dict(body) if request.call_type == Some("pass_through_endpoint") => {
            (Some(prompt_value()), pass_through_output(body))
        }
        ResponseObj::Embedding | ResponseObj::List(_) | ResponseObj::Dict(_) => (None, None),
    }
}

fn pass_through_output(body: &Json) -> Option<Json> {
    match body {
        Json::Object(fields) => match fields.get("response") {
            Some(Json::Null) => None,
            Some(value) => Some(value.clone()),
            None => Some(Json::Text(String::new())),
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
