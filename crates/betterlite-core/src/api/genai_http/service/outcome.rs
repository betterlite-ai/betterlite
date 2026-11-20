use std::iter;
use std::sync::Arc;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use futures_util::stream::BoxStream;
use http::HeaderMap as HttpHeaders;

use super::super::stream::{StreamInputs, StreamSettings, stream_metadata};
use super::{AsyncStream, SyncStream};
use crate::google_genai::streaming::{AsyncGoogleGenAiStream, GoogleGenAiStream};

// The sync iterator reads the buffered body as one chunk and finds the events in it
pub(super) fn sync_stream(
    inputs: &StreamInputs<'_>,
    settings: &StreamSettings,
    answer: UpstreamResponse,
) -> SyncStream {
    let metadata = stream_metadata(inputs, &answer.headers);
    let iterator = GoogleGenAiStream::new(iter::once(Ok(answer.body)), settings.line_limit);
    SyncStream { iterator, metadata }
}

pub(super) fn async_stream(
    inputs: &StreamInputs<'_>,
    settings: &StreamSettings,
    headers: &HttpHeaders,
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
) -> AsyncStream {
    let metadata = stream_metadata(inputs, headers);
    let iterator = AsyncGoogleGenAiStream::new(
        body,
        metadata.clone(),
        settings.logging.clone(),
        Arc::clone(&settings.clock),
        settings.line_limit,
    );
    AsyncStream { iterator, metadata }
}
