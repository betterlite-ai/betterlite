use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::blocked_usage::{
    OriginalResponse, blocked_chat_stream_usage, blocked_response_usage,
    blocked_responses_api_usage, blocked_responses_stream_usage,
};
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Shape {
    Response,
    ResponsesApi,
    ChatStream,
    ResponsesStream,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Entry {
    Json { value: GuardrailValue },
    Bytes { text: String },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Original {
    Absent,
    Response { response: Object },
    Stream { items: Vec<Entry> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    shape: Shape,
    original: Original,
}

fn stream_item(entry: &Entry) -> StreamItem {
    match entry {
        Entry::Json { value } => StreamItem::Json(value.clone()),
        Entry::Bytes { text } => StreamItem::Bytes(text.clone().into_bytes()),
    }
}

fn original_response(original: &Original) -> OriginalResponse {
    match original {
        Original::Absent => OriginalResponse::Absent,
        Original::Response { response } => OriginalResponse::Response(response.clone()),
        Original::Stream { items } => {
            OriginalResponse::Stream(items.iter().map(stream_item).collect())
        }
    }
}

pub(super) fn blocked_usage(request: &Request) -> Value {
    let original = original_response(&request.original);
    let responses = match request.shape {
        Shape::Response => {
            let usage = blocked_response_usage(&original);
            return json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
            });
        }
        Shape::ChatStream => {
            let (prompt_tokens, completion_tokens) = blocked_chat_stream_usage(&original);
            return json!({
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
            });
        }
        Shape::ResponsesApi => blocked_responses_api_usage(&original),
        Shape::ResponsesStream => blocked_responses_stream_usage(&original),
    };
    json!({
        "input_tokens": responses.input_tokens,
        "output_tokens": responses.output_tokens,
        "total_tokens": responses.total_tokens,
    })
}
