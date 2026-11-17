use std::time::Duration;

use betterlite_models::decisions::Json;
use futures_util::stream::BoxStream;
use http::HeaderMap;

use crate::message_utilities::response_streams::StreamError;
use crate::operation::text_completion::TextCompletionFailure;

pub struct TextStreamAnswer {
    pub body: BoxStream<'static, Result<Json, TextCompletionFailure>>,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

// The text chunks the route frames as SSE, already run through B5366's text completion wrapper.
pub struct TextStreamResult {
    pub body: BoxStream<'static, Result<Json, StreamError>>,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}
