use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::model_response::content_from_dict_response;
use super::{GalileoKwargs, ResponseObj, langfuse_style_rerank_prompt, serialize_galileo_output};
use crate::python_json;

fn rerank_content(kwargs: &GalileoKwargs, results: &Json) -> (String, String, Json) {
    let rerank_prompt = langfuse_style_rerank_prompt(kwargs);
    let input_text = python_json::dumps_object(&rerank_prompt);
    (
        input_text,
        serialize_galileo_output(Some(results)),
        Json::Object(rerank_prompt),
    )
}

fn responses_api_content(
    prompt: &IndexMap<String, Json>,
    kwargs: &GalileoKwargs,
    output: &Json,
) -> (String, String, Json) {
    let present_output = super::super::support::has_value(output).then_some(output);
    (
        super::prompt_to_input_text(prompt),
        serialize_galileo_output(present_output),
        super::messages_or_empty(kwargs),
    )
}

fn realtime_list_content(kwargs: &GalileoKwargs, items: &[Json]) -> (String, String, Json) {
    let input_val = kwargs.input.clone().unwrap_or(Json::Null);
    let output_val = Json::List(items.to_vec());
    (
        serialize_galileo_output(Some(&input_val)),
        serialize_galileo_output(Some(&output_val)),
        input_val,
    )
}

// The pass-through dict's "response" key, a generic dict read through content_from_dict_response, or neither.
enum DictKind<'val> {
    PassThrough(&'val IndexMap<String, Json>),
    Generic(&'val IndexMap<String, Json>),
    Unknown,
}

fn dict_response_content(
    prompt: &IndexMap<String, Json>,
    kwargs: &GalileoKwargs,
    kind: &DictKind<'_>,
) -> (String, String, Json) {
    match kind {
        DictKind::PassThrough(value) => (
            super::prompt_to_input_text(prompt),
            serialize_galileo_output(value.get("response")),
            Json::Object(prompt.clone()),
        ),
        DictKind::Generic(value) => (
            super::prompt_to_input_text(prompt),
            content_from_dict_response(value),
            super::messages_or_empty(kwargs),
        ),
        DictKind::Unknown => (
            super::prompt_to_input_text(prompt),
            String::new(),
            super::messages_or_empty(kwargs),
        ),
    }
}

// The families whose messages_for_span is always the echoed request prompt, never the chat history.
enum MarkerFamily<'val> {
    Embedding,
    Speech,
    Image(Option<&'val Json>),
    Transcription(Option<&'val String>),
}

fn marker_family_content(
    prompt: &IndexMap<String, Json>,
    family: &MarkerFamily<'_>,
) -> (String, String, Json) {
    let output_text = match family {
        MarkerFamily::Embedding => "embedding-output".to_owned(),
        MarkerFamily::Speech => "speech-output".to_owned(),
        MarkerFamily::Image(data) => serialize_galileo_output(*data),
        MarkerFamily::Transcription(text) => {
            serialize_galileo_output(text.cloned().map(Json::Text).as_ref())
        }
    };
    (
        super::prompt_to_input_text(prompt),
        output_text,
        Json::Object(prompt.clone()),
    )
}

pub(super) fn response_content(
    prompt: &IndexMap<String, Json>,
    kwargs: &GalileoKwargs,
    response: &ResponseObj,
) -> (String, String, Json) {
    match response {
        ResponseObj::Embedding => marker_family_content(prompt, &MarkerFamily::Embedding),
        ResponseObj::Speech => marker_family_content(prompt, &MarkerFamily::Speech),
        ResponseObj::Image { data } => {
            marker_family_content(prompt, &MarkerFamily::Image(data.as_ref()))
        }
        ResponseObj::Transcription { text } => {
            marker_family_content(prompt, &MarkerFamily::Transcription(text.as_ref()))
        }
        ResponseObj::Chat { message } => (
            super::prompt_to_input_text(prompt),
            serialize_galileo_output(message.as_ref()),
            super::messages_or_empty(kwargs),
        ),
        ResponseObj::TextCompletion { text } => (
            super::prompt_to_input_text(prompt),
            serialize_galileo_output(text.clone().map(Json::Text).as_ref()),
            super::messages_or_empty(kwargs),
        ),
        ResponseObj::Rerank { results } => rerank_content(kwargs, results),
        ResponseObj::ResponsesApi { output } => responses_api_content(prompt, kwargs, output),
        ResponseObj::RealtimeList { items } => realtime_list_content(kwargs, items),
        ResponseObj::PassThroughDict { value } => {
            dict_response_content(prompt, kwargs, &DictKind::PassThrough(value))
        }
        ResponseObj::GenericDict { value } => {
            dict_response_content(prompt, kwargs, &DictKind::Generic(value))
        }
        ResponseObj::Unknown => dict_response_content(prompt, kwargs, &DictKind::Unknown),
    }
}
