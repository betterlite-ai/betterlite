#![cfg(test)]

use std::sync::{Arc, Mutex};

use aws_sdk_bedrockruntime::types::{
    BidirectionalInputPayloadPart, BidirectionalOutputPayloadPart,
    InvokeModelWithBidirectionalStreamInput as InputEvent,
    InvokeModelWithBidirectionalStreamOutput as OutputEvent,
};
use aws_smithy_types::Blob;
use betterlite_models::decisions::Json;
use futures_util::stream::{StreamExt as _, iter};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc;

use super::{InputQueue, OutputStream, SdkInput, SdkOutput};
use crate::realtime_events::stream::{
    BedrockInputStream as _, BedrockOutputStream as _, BedrockPayloadPart as _, StreamError,
};
use crate::realtime_transport::error::ProviderClass as _;
use crate::realtime_transport::runtime::error::SdkStreamError;

#[tokio::test]
async fn sdk_input_sends_the_event_bytes_and_refuses_after_close() {
    let (sender, mut receiver) = mpsc::channel(1);
    let input = SdkInput {
        sender: Mutex::new(Some(sender)),
    };
    input.send(Json::Text("hello".to_owned())).await.unwrap();
    let InputEvent::Chunk(part) = receiver.recv().await.unwrap().unwrap() else {
        panic!("the request direction carries chunks");
    };
    let bytes = part
        .bytes
        .map(Blob::into_inner)
        .expect("the chunk carries bytes");
    assert_eq!(
        serde_json::from_slice::<Json>(&bytes).unwrap(),
        Json::Text("hello".to_owned())
    );
    input.close().await.unwrap();
    assert!(
        input.send(Json::Null).await.is_err(),
        "a closed input refuses events"
    );
    assert!(
        receiver.recv().await.is_none(),
        "the close ends the request direction"
    );
}

#[tokio::test]
async fn sdk_input_queue_yields_queued_events_then_ends() {
    let (sender, receiver) = mpsc::channel(1);
    let mut queue = Box::pin(InputQueue {
        receiver: Mutex::new(receiver),
    });
    let event = InputEvent::Chunk(BidirectionalInputPayloadPart::builder().build());
    sender.send(Ok(event)).await.unwrap();
    let first = queue.next().await.expect("a queued event").unwrap();
    assert!(matches!(first, InputEvent::Chunk(_)));
    drop(sender);
    assert!(
        queue.next().await.is_none(),
        "an ended sender ends the request direction"
    );
}

#[tokio::test]
async fn sdk_output_receives_chunks_and_ends() {
    let events: Vec<Result<Option<OutputEvent>, StreamError<SdkStreamError>>> = vec![
        Ok(Some(OutputEvent::Chunk(
            BidirectionalOutputPayloadPart::builder()
                .bytes(Blob::new(b"payload".to_vec()))
                .build(),
        ))),
        Ok(Some(OutputEvent::Unknown)),
        Ok(None),
    ];
    let stream: OutputStream = Box::pin(iter(events));
    let output = SdkOutput {
        output: Arc::new(AsyncMutex::new(stream)),
    };
    let chunk = output.receive().await.unwrap().expect("the chunk arrives");
    let part = chunk.value().expect("the chunk carries a payload part");
    assert_eq!(part.bytes(), Some(b"payload".as_slice()));
    let unknown = output
        .receive()
        .await
        .unwrap()
        .expect("an unknown event still arrives");
    assert!(
        unknown.value().is_none(),
        "an unknown event carries no part"
    );
    assert!(
        output.receive().await.unwrap().is_none(),
        "the ended stream ends the output"
    );
}

#[test]
fn sdk_stream_errors_report_the_service_class_and_message() {
    let failure = SdkStreamError::Provider {
        class_name: "AccessDeniedException".to_owned(),
        message: "the fixture denied it".to_owned(),
    };
    assert_eq!(failure.provider_class_name(), Some("AccessDeniedException"));
    assert_eq!(failure.provider_message(), Some("the fixture denied it"));
    assert_eq!(SdkStreamError::Closed.provider_class_name(), None);
    assert_eq!(SdkStreamError::Closed.provider_message(), None);
}
