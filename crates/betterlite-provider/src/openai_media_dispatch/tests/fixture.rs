use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::videos::VideoOptions;
use indexmap::IndexMap;

use crate::openai_media_dispatch::OpenAiMediaResult;
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::operation::OperationResult;

pub(crate) use crate::openai_media_dispatch::test_support::{Fixture, Recorder};

pub(crate) fn media_result(
    outcome: Result<OperationResult, OpenAiMediaError>,
) -> OpenAiMediaResult {
    match outcome {
        Ok(OperationResult::OpenAiMedia(media)) => media,
        Ok(_) => panic!("the media dispatcher returns a media result"),
        Err(error) => panic!("the media dispatch failed: {error}"),
    }
}

pub(crate) fn media_error(result: Result<OperationResult, OpenAiMediaError>) -> OpenAiMediaError {
    match result {
        Ok(_) => panic!("the dispatch should fail"),
        Err(error) => error,
    }
}

pub(crate) fn json_body(request: &OutboundRequest) -> IndexMap<String, Json> {
    serde_json::from_slice(json_bytes(request)).expect("the request body is a JSON object")
}

pub(crate) fn parts(request: &OutboundRequest) -> &[MultipartPart] {
    let RequestBody::Multipart { parts, .. } = &request.body else {
        panic!("the request body is multipart");
    };
    parts
}

pub(crate) fn boundary(request: &OutboundRequest) -> &str {
    let RequestBody::Multipart { boundary, .. } = &request.body else {
        panic!("the request body is multipart");
    };
    boundary
}

pub(crate) fn options(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("the options are a JSON object")
}

pub(crate) fn video_options(source: &str) -> VideoOptions {
    serde_json::from_str(source).expect("the options are a JSON object")
}

fn json_bytes(request: &OutboundRequest) -> &[u8] {
    let RequestBody::Json(bytes) = &request.body else {
        panic!("the request body is JSON");
    };
    bytes
}
