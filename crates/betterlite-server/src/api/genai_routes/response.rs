use axum::Json;
use axum::body::Body;
use axum::http::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_core::api::genai_chat_output::GenAiOutputError;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};

use super::ports::{GenerationAnswer, GenerationBody, PromptTokenDetail, TokenCountResponse};

// the pipeline's answer: its headers plus either the JSON document or the raw SSE chunks
pub(super) fn generation(answer: GenerationAnswer) -> Response {
    let GenerationAnswer { headers, body } = answer;
    match body {
        GenerationBody::Json(json) => {
            let mut response = Json(json).into_response();
            merge_headers(response.headers_mut(), &headers);
            response
        }
        GenerationBody::Events(events) => event_response(events, &headers),
    }
}

// create_response(media_type="text/event-stream", headers=_sse_stream_headers(custom_headers))
fn event_response(
    events: BoxStream<'static, Result<Vec<u8>, GenAiOutputError>>,
    headers: &HeaderMap,
) -> Response {
    let mut response = Response::new(Body::from_stream(events));
    let all = response.headers_mut();
    merge_headers(all, headers);
    all.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    all.insert("x-accel-buffering", HeaderValue::from_static("no"));
    if !all.contains_key(CONTENT_TYPE) {
        all.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
    }
    response
}

// the three branches of google_count_tokens, in LiteLLM's order; an original_response with no field is the second
pub(super) fn token_count(response: Option<TokenCountResponse>) -> Response {
    let details = match response {
        Some(counted) => match counted.original_response {
            Some(original)
                if original.total_tokens.is_some() || original.prompt_tokens_details.is_some() =>
            {
                TokenCountDetails {
                    total_tokens: original.total_tokens.unwrap_or(0),
                    prompt_tokens_details: original.prompt_tokens_details.unwrap_or_default(),
                }
            }
            Some(_empty) | None => TokenCountDetails {
                total_tokens: counted.total_tokens,
                prompt_tokens_details: Vec::new(),
            },
        },
        None => TokenCountDetails {
            total_tokens: 0,
            prompt_tokens_details: Vec::new(),
        },
    };
    Json(details).into_response()
}

// TokenCountDetailsResponse: both fields always present, promptTokensDetails never null
#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TokenCountDetails {
    pub(super) total_tokens: i64,
    pub(super) prompt_tokens_details: Vec<PromptTokenDetail>,
}

#[derive(Serialize)]
struct DetailBody {
    detail: &'static str,
}

// Starlette's own answer to a path no route matched
pub(super) fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(DetailBody {
            detail: "Not Found",
        }),
    )
        .into_response()
}

// Starlette's own answer to a route matched under another method
pub(super) fn method_not_allowed() -> Response {
    let mut response = (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(DetailBody {
            detail: "Method Not Allowed",
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(ALLOW, HeaderValue::from_static("POST"));
    response
}

fn merge_headers(target: &mut HeaderMap, source: &HeaderMap) {
    for (name, value) in source {
        target.insert(name.clone(), value.clone());
    }
}
