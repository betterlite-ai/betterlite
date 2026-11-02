use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::model_response::{
    content_from_dict_response, image_object_model_dump, message_model_dump,
};
use super::responses_output::output_item_dump;
use super::{GalileoKwargs, ResponseObj, langfuse_style_rerank_prompt, serialize_galileo_output};
use crate::python_json;

fn standard_content(
    prompt: &IndexMap<String, Json>,
    output_text: String,
    messages_for_span: Json,
) -> (String, String, Json) {
    (
        super::prompt_to_input_text(prompt),
        output_text,
        messages_for_span,
    )
}

fn rerank_content(kwargs: &GalileoKwargs, results: &Json) -> (String, String, Json) {
    let rerank_prompt = langfuse_style_rerank_prompt(kwargs);
    let input_text = python_json::dumps_object(&rerank_prompt);
    let output_text = serialize_galileo_output(Some(results));
    (input_text, output_text, Json::Object(rerank_prompt))
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
        MarkerFamily::Image(data) => serialize_galileo_output(
            data.map(|value| super::project_list_or_passthrough(value, image_object_model_dump))
                .as_ref(),
        ),
        MarkerFamily::Transcription(text) => {
            serialize_galileo_output(text.cloned().map(Json::Text).as_ref())
        }
    };
    standard_content(prompt, output_text, Json::Object(prompt.clone()))
}

// litellm only takes the realtime-list branch when call_type == "_arealtime" (galileo.py:601); any other call_type falls through to the final fallback, which an Unknown response also takes.
fn realtime_list_or_fallback(
    prompt: &IndexMap<String, Json>,
    kwargs: &GalileoKwargs,
    items: &[Json],
) -> (String, String, Json) {
    if kwargs.call_type.as_deref() == Some("_arealtime") {
        return realtime_list_content(kwargs, items);
    }
    standard_content(prompt, String::new(), super::messages_or_empty(kwargs))
}

// litellm only takes the pass-through branch when call_type == "pass_through_endpoint" (galileo.py:609); every other call_type, and a plain GenericDict response, reads straight out of the raw dict instead.
fn pass_through_or_generic(
    prompt: &IndexMap<String, Json>,
    kwargs: &GalileoKwargs,
    value: &IndexMap<String, Json>,
) -> (String, String, Json) {
    match kwargs.call_type.as_deref() {
        Some("pass_through_endpoint") => standard_content(
            prompt,
            serialize_galileo_output(value.get("response")),
            Json::Object(prompt.clone()),
        ),
        _ => standard_content(
            prompt,
            content_from_dict_response(value),
            super::messages_or_empty(kwargs),
        ),
    }
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
        ResponseObj::Chat { message } => standard_content(
            prompt,
            serialize_galileo_output(message.as_ref().map(message_model_dump).as_ref()),
            super::messages_or_empty(kwargs),
        ),
        ResponseObj::TextCompletion { text } => standard_content(
            prompt,
            serialize_galileo_output(text.clone().map(Json::Text).as_ref()),
            super::messages_or_empty(kwargs),
        ),
        ResponseObj::Rerank { results } => rerank_content(kwargs, results),
        // The Responses API echoes its own output back, gated by has_value like Python's `if response is not None`; each item goes through its parsed pydantic shape first, the way ResponsesAPIResponse.output already parsed it.
        ResponseObj::ResponsesApi { output } => {
            let projected_output = super::project_list_or_passthrough(output, output_item_dump);
            let present_output =
                super::super::support::has_value(&projected_output).then_some(&projected_output);
            standard_content(
                prompt,
                serialize_galileo_output(present_output),
                super::messages_or_empty(kwargs),
            )
        }
        ResponseObj::RealtimeList { items } => realtime_list_or_fallback(prompt, kwargs, items),
        ResponseObj::PassThroughDict { value } | ResponseObj::GenericDict { value } => {
            pass_through_or_generic(prompt, kwargs, value)
        }
        ResponseObj::Unknown => {
            standard_content(prompt, String::new(), super::messages_or_empty(kwargs))
        }
    }
}
