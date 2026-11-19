use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use async_trait::async_trait;
use aws_sdk_bedrockruntime::Client;
// ast-grep-ignore: no-long-names
use aws_sdk_bedrockruntime::types::error::InvokeModelWithBidirectionalStreamInputError as InputError;
use aws_sdk_bedrockruntime::types::{
    BidirectionalInputPayloadPart,
    // ast-grep-ignore: no-long-names
    InvokeModelWithBidirectionalStreamInput as InputEvent,
    // ast-grep-ignore: no-long-names
    InvokeModelWithBidirectionalStreamOutput as OutputEvent,
};
use aws_smithy_http::event_stream::EventStreamSender;
use aws_smithy_types::Blob;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use futures_util::stream::{Stream, StreamExt as _, unfold};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc;

use super::error::{SdkStreamError, closed_stream, provider_error};
use crate::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    BedrockPayloadPart, StreamError,
};

// The SDK receiver's type is private to the crate, so it is erased behind this event stream
type OutputStream =
    Pin<Box<dyn Stream<Item = Result<Option<OutputEvent>, StreamError<SdkStreamError>>> + Send>>;

// One session's SDK stream: the input queue feeds the request direction, the stream drains the response
pub(super) struct SdkStream {
    input: SdkInput,
    output: Arc<AsyncMutex<OutputStream>>,
}

impl SdkStream {
    pub(super) async fn open(
        client: &Client,
        model_id: &str,
    ) -> Result<Self, StreamError<SdkStreamError>> {
        let (sender, receiver) = mpsc::channel(1);
        let input = SdkInput {
            sender: Mutex::new(Some(sender)),
        };
        let body = EventStreamSender::from(InputQueue {
            receiver: Mutex::new(receiver),
        });
        let output = client
            .invoke_model_with_bidirectional_stream()
            .model_id(model_id.to_owned())
            .body(body)
            .send()
            .await
            .map_err(|error| provider_error(&error))?;
        let events = output.body;
        let stream = unfold(events, |mut responses| async move {
            let event = responses
                .recv()
                .await
                .map_err(|error| provider_error(&error));
            Some((event, responses))
        });
        Ok(Self {
            input,
            output: Arc::new(AsyncMutex::new(Box::pin(stream))),
        })
    }
}

#[async_trait]
impl BedrockBidirectionalStream for SdkStream {
    type Error = SdkStreamError;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = SdkStreamError> {
        &self.input
    }

    async fn await_output(
        &self,
    ) -> Result<
        (Json, Box<dyn BedrockOutputStream<Error = SdkStreamError>>),
        StreamError<SdkStreamError>,
    > {
        let output: Box<dyn BedrockOutputStream<Error = SdkStreamError>> = Box::new(SdkOutput {
            output: Arc::clone(&self.output),
        });
        Ok((Json::Null, output))
    }
}

struct SdkInput {
    sender: Mutex<Option<mpsc::Sender<Result<InputEvent, InputError>>>>,
}

#[async_trait]
impl BedrockInputStream for SdkInput {
    type Error = SdkStreamError;

    async fn send(&self, event: Json) -> Result<(), StreamError<SdkStreamError>> {
        let text = python_json::dumps(&event);
        let part = BidirectionalInputPayloadPart::builder()
            .bytes(Blob::new(text.into_bytes()))
            .build();
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(open) = sender else {
            return Err(closed_stream());
        };
        open.send(Ok(InputEvent::Chunk(part)))
            .await
            .map_err(|_closed| closed_stream())
    }

    async fn close(&self) -> Result<(), StreamError<SdkStreamError>> {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(sender);
        Ok(())
    }
}

// The SDK polls the request body; an ended or dropped sender ends the request direction
struct InputQueue {
    receiver: Mutex<mpsc::Receiver<Result<InputEvent, InputError>>>,
}

impl Stream for InputQueue {
    type Item = Result<InputEvent, InputError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let receiver = self
            .get_mut()
            .receiver
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        receiver.poll_recv(cx)
    }
}

struct SdkOutput {
    output: Arc<AsyncMutex<OutputStream>>,
}

#[async_trait]
impl BedrockOutputStream for SdkOutput {
    type Error = SdkStreamError;

    async fn receive(
        &self,
    ) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<SdkStreamError>> {
        let mut output = self.output.lock().await;
        match output.as_mut().next().await {
            None | Some(Ok(None)) => Ok(None),
            Some(Err(error)) => Err(error),
            Some(Ok(Some(event))) => {
                let chunk: Box<dyn BedrockOutputChunk> = Box::new(SdkOutputChunk::new(event));
                Ok(Some(chunk))
            }
        }
    }
}

struct SdkOutputChunk {
    part: Option<SdkPayloadPart>,
}

impl SdkOutputChunk {
    fn new(event: OutputEvent) -> Self {
        if let OutputEvent::Chunk(part) = event {
            Self {
                part: Some(SdkPayloadPart {
                    bytes: part.bytes.map(Blob::into_inner),
                }),
            }
        } else {
            Self { part: None }
        }
    }
}

impl BedrockOutputChunk for SdkOutputChunk {
    fn value(&self) -> Option<&dyn BedrockPayloadPart> {
        let part: &dyn BedrockPayloadPart = self.part.as_ref()?;
        Some(part)
    }
}

struct SdkPayloadPart {
    bytes: Option<Vec<u8>>,
}

impl BedrockPayloadPart for SdkPayloadPart {
    fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }
}

mod tests;
