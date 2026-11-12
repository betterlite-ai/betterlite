use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::galileo_payloads::content::ResponseObj;

// litellm.integrations.galileo.GalileoObserve._get_galileo_input_output_content dispatches on the response type it was handed; the payload keeps no type tag, so the call type names the family.
pub(super) fn response_object(call_type: &str, response: &Json) -> ResponseObj {
    match call_type {
        "embedding" | "aembedding" => ResponseObj::Embedding,
        "speech" | "aspeech" => ResponseObj::Speech,
        "image_generation" | "aimage_generation" | "image_edit" | "aimage_edit" => {
            ResponseObj::Image {
                data: object_field(response, "data").cloned(),
            }
        }
        "transcription" | "atranscription" => ResponseObj::Transcription {
            text: text_field(response),
        },
        "rerank" | "arerank" => ResponseObj::Rerank {
            results: object_field(response, "results")
                .cloned()
                .unwrap_or(Json::Null),
        },
        "responses" | "aresponses" => ResponseObj::ResponsesApi {
            output: object_field(response, "output")
                .cloned()
                .unwrap_or(Json::Null),
        },
        "_arealtime" => realtime(response),
        "pass_through_endpoint" => pass_through(response),
        _ => completion(response),
    }
}

fn realtime(response: &Json) -> ResponseObj {
    let Json::List(items) = response else {
        return ResponseObj::Unknown;
    };
    ResponseObj::RealtimeList {
        items: items.clone(),
    }
}

fn pass_through(response: &Json) -> ResponseObj {
    let Json::Object(value) = response else {
        return ResponseObj::Unknown;
    };
    ResponseObj::PassThroughDict {
        value: value.clone(),
    }
}

fn completion(response: &Json) -> ResponseObj {
    let Json::Object(fields) = response else {
        return ResponseObj::Unknown;
    };
    let Some(choice) = first_choice(fields) else {
        return ResponseObj::GenericDict {
            value: fields.clone(),
        };
    };
    if let Some(message) = choice.get("message") {
        return ResponseObj::Chat {
            message: Some(message.clone()),
        };
    }
    if let Some(Json::Text(text)) = choice.get("text") {
        return ResponseObj::TextCompletion {
            text: Some(text.clone()),
        };
    }
    ResponseObj::GenericDict {
        value: fields.clone(),
    }
}

fn first_choice(fields: &IndexMap<String, Json>) -> Option<&IndexMap<String, Json>> {
    let Some(Json::List(choices)) = fields.get("choices") else {
        return None;
    };
    match choices.first() {
        Some(Json::Object(choice)) => Some(choice),
        _ => None,
    }
}

fn object_field<'json>(response: &'json Json, name: &str) -> Option<&'json Json> {
    match response {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn text_field(response: &Json) -> Option<String> {
    match object_field(response, "text") {
        Some(Json::Text(text)) => Some(text.clone()),
        _ => None,
    }
}
