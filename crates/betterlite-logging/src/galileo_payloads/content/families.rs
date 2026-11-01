use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::model_response::content_from_dict_response;
use super::{GalileoKwargs, ResponseObj, langfuse_style_rerank_prompt, serialize_galileo_output};
use crate::python_json;

fn rerank_content(kwargs: &GalileoKwargs, results: &Json) -> (String, String, Json) {
    let rerank_prompt = langfuse_style_rerank_prompt(kwargs);
    let input_text = python_json::dumps_object(&rerank_prompt);
    let output_text = serialize_galileo_output(Some(results));
    (input_text, output_text, Json::Object(rerank_prompt))
}

// Every family but rerank and the realtime list reads its input text from the same echoed prompt.
fn echoed_prompt_triple(
    prompt: &IndexMap<String, Json>,
    output_text: String,
    messages_for_span: Json,
) -> (String, String, Json) {
    let input_text = super::prompt_to_input_text(prompt);
    (input_text, output_text, messages_for_span)
}

fn realtime_list_content(kwargs: &GalileoKwargs, items: &[Json]) -> (String, String, Json) {
    let input_val = kwargs.input.clone().unwrap_or(Json::Null);
    let output_text = serialize_galileo_output(Some(&Json::List(items.to_vec())));
    (
        serialize_galileo_output(Some(&input_val)),
        output_text,
        input_val,
    )
}

// The Responses API echoes its own output back, gated by has_value like Python's `if response is not None`.
fn responses_api_content(
    prompt: &IndexMap<String, Json>,
    kwargs: &GalileoKwargs,
    output: &Json,
) -> (String, String, Json) {
    let present_output = super::super::support::has_value(output).then_some(output);
    echoed_prompt_triple(
        prompt,
        serialize_galileo_output(present_output),
        super::messages_or_empty(kwargs),
    )
}

fn pass_through_dict_content(
    prompt: &IndexMap<String, Json>,
    value: &IndexMap<String, Json>,
) -> (String, String, Json) {
    let output_text = serialize_galileo_output(value.get("response"));
    echoed_prompt_triple(prompt, output_text, Json::Object(prompt.clone()))
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
    echoed_prompt_triple(prompt, output_text, Json::Object(prompt.clone()))
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
        ResponseObj::Chat { message } => echoed_prompt_triple(
            prompt,
            serialize_galileo_output(message.as_ref()),
            super::messages_or_empty(kwargs),
        ),
        ResponseObj::TextCompletion { text } => echoed_prompt_triple(
            prompt,
            serialize_galileo_output(text.clone().map(Json::Text).as_ref()),
            super::messages_or_empty(kwargs),
        ),
        ResponseObj::Rerank { results } => rerank_content(kwargs, results),
        ResponseObj::ResponsesApi { output } => responses_api_content(prompt, kwargs, output),
        ResponseObj::RealtimeList { items } => realtime_list_content(kwargs, items),
        ResponseObj::PassThroughDict { value } => pass_through_dict_content(prompt, value),
        ResponseObj::GenericDict { value } => echoed_prompt_triple(
            prompt,
            content_from_dict_response(value),
            super::messages_or_empty(kwargs),
        ),
        ResponseObj::Unknown => {
            echoed_prompt_triple(prompt, String::new(), super::messages_or_empty(kwargs))
        }
    }
}
