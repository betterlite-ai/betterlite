use std::pin::Pin;
use std::task::{Context, Poll};

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use futures_util::Stream;

use crate::bedrock_http_stream::stream::StreamFailure;
use crate::event_frames::error::EventFrameError;
use crate::invoke_events::InvokeChunk;
use crate::invoke_events::error::InvokeStreamError;

use super::WrappedStream;

impl Stream for WrappedStream<'_> {
    type Item = Result<InvokeChunk, ErrorClass>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let stream = self.get_mut();
        Pin::new(&mut stream.completion_stream)
            .poll_next(cx)
            .map(|item| item.map(|chunk| chunk.map_err(|source| classify(&source, stream))))
    }
}

// The decoder reports provider statuses; CustomStreamWrapper applies the shared exception taxonomy only when consumed.
fn classify(source: &StreamFailure, stream: &WrappedStream<'_>) -> ErrorClass {
    let message =
        if let StreamFailure::Decode(InvokeStreamError::Frame(EventFrameError::Bedrock {
            error,
            ..
        })) = source
        {
            error.error.message.clone()
        } else {
            source.to_string()
        };
    let raw = RawException {
        model: stream.model.clone(),
        custom_llm_provider: stream.custom_llm_provider.to_owned(),
        error_str: message.clone(),
        status_code: source.status(),
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    };
    dispatch(&DispatchInput::Raw(Box::new(raw)), stream.redactor)
}
