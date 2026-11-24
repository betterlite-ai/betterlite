use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse as UpstreamEvent;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

// The Responses stream as the sync creation call hands it over: one typed event per pull, None at the end.
pub trait SyncResponsesEventStream: Send {
    type Error: StdError + Send + Sync + 'static;

    fn next_event(&mut self) -> Option<Result<UpstreamEvent, Self::Error>>;
}

// The Responses stream as the awaited creation call hands it over, awaited once per event.
#[async_trait]
pub trait AsyncResponsesEventStream: Send {
    type Error: StdError + Send + Sync + 'static;

    async fn next_event(&mut self) -> Option<Result<UpstreamEvent, Self::Error>>;
}

// LiteLLM's responses() returns a ResponsesAPIResponse or a BaseResponsesAPIStreamingIterator, split so the wrap is the type check.
pub enum ResponsesAnswer<Stream> {
    Complete(Box<ResponsesApiResponse>),
    Stream(Stream),
}

// The sync creation call's answer: the stream is the one its iterator hands over.
pub type SyncResponsesAnswer<Error> =
    ResponsesAnswer<Box<dyn SyncResponsesEventStream<Error = Error>>>;

// The awaited creation call's answer: the same two shapes over the async stream.
pub type AsyncResponsesAnswer<Error> =
    ResponsesAnswer<Box<dyn AsyncResponsesEventStream<Error = Error>>>;
