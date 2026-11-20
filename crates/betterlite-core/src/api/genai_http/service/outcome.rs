use std::iter;
use std::sync::Arc;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use futures_util::stream::BoxStream;
use http::HeaderMap as HttpHeaders;

use super::super::config::GenerateContentHttpConfig;
use super::super::error::GenAiHttpError;
use super::super::stream::{StreamInputs, StreamSettings, stream_metadata};
use super::{Answer, AsyncStream, GenerateContentOutcome, SyncStream};
use crate::google_genai::streaming::{AsyncGoogleGenAiStream, GoogleGenAiStream};

// The sync handler's outcome: the response it already transformed, or the iterator over a buffered reply
pub(super) fn sync_outcome<C: GenerateContentHttpConfig>(
    answer: Answer<'_, '_, C>,
) -> Result<GenerateContentOutcome<C::Response>, GenAiHttpError<C::Error>> {
    match answer {
        Answer::Response(response) => Ok(GenerateContentOutcome::Response(response)),
        Answer::SyncEvents(wire, reply) => {
            let iterator = sync_stream(&wire.inputs, wire.settings, reply);
            Ok(GenerateContentOutcome::Stream(Box::new(iterator)))
        }
        Answer::AsyncEvents(_, _) => Err(GenAiHttpError::UnexpectedStream),
    }
}

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
