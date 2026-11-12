use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use betterlite_vertex::multimodal_embedding_records::error::INTERNAL_SERVER_ERROR;
use betterlite_vertex::multimodal_embedding_records::error::MultimodalEmbeddingError;
use betterlite_vertex::multimodal_embedding_records::request::transform_request;
use betterlite_vertex::multimodal_embedding_records::response::MultimodalEmbeddingAnswer;
use betterlite_vertex::multimodal_embedding_records::response::transform_response;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    requests: Vec<RequestCall>,
    replies: Vec<ReplyCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    input: Json,
    optional: Extra,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    model: String,
    status: u16,
    response: Json,
    input: Json,
    optional: Extra,
}

pub(super) fn run(input: &Input) -> Value {
    json!({
        "requests": input.requests.iter().map(request_entry).collect::<Vec<_>>(),
        "replies": input.replies.iter().map(reply_entry).collect::<Vec<_>>(),
    })
}

fn request_entry(call: &RequestCall) -> Value {
    transform_request(&call.input, &call.optional)
        .map_or_else(|_| json!({"error": true}), |body| json!(body))
}

fn reply_entry(call: &ReplyCall) -> Value {
    let Ok(request) = transform_request(&call.input, &call.optional) else {
        return json!({"error": true});
    };
    let Ok(body) = serde_json::to_vec(&call.response) else {
        return json!({"error": true});
    };
    match transform_response(call.status, &body, &request, &call.model) {
        Ok(answer) => answer_value(&answer),
        Err(MultimodalEmbeddingError::MissingPredictions) => json!({
            "error": {
                "status": 500,
                "type": INTERNAL_SERVER_ERROR,
                "param": null,
                "code": null,
            },
        }),
        Err(_) => json!({"error": true}),
    }
}

fn answer_value(answer: &MultimodalEmbeddingAnswer) -> Value {
    json!({
        "model": answer.response.model,
        "data": answer.response.data,
        "usage": {
            "prompt_tokens": answer.response.usage.prompt_tokens,
            "completion_tokens": answer.response.usage.completion_tokens,
            "total_tokens": answer.response.usage.total_tokens,
            "prompt_tokens_details": {
                "character_count": answer.details.character_count,
                "image_count": answer.details.image_count,
                "video_length_seconds": answer.details.video_length_seconds.to_string(),
            },
        },
    })
}
