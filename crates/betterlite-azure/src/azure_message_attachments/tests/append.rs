#![cfg(test)]

use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::threads::error::{ReplyFault, ThreadsError};
use betterlite_openai::threads::records::{MessageStatus, ThreadMessage};

use crate::azure_message_attachments::error::MessagesError;
use crate::azure_message_attachments::operations::{MessageCall, MessageReply};

use super::support::{
    Auth, Fixture, KEY, MESSAGE, MESSAGE_ATTACHMENTS, MESSAGE_DATA, MESSAGE_EXTRA,
    MESSAGE_IN_PROGRESS, MESSAGE_NO_STATUS, MESSAGE_NULL_STATUS, MESSAGES_URL, data, harness, wire,
};

const MESSAGE_EMPTY_STATUS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"user","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":""}"#;
const MESSAGE_UNKNOWN_STATUS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"user","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"queued"}"#;
const QUOTED_URL: &str =
    "https://example.test/openai/threads/thread%20a%2Fb%3Fc/messages?api-version=2024-02-01";

fn expected(json: &str) -> ThreadMessage {
    serde_json::from_str(json).unwrap()
}

pub(super) async fn check() {
    status_preserved().await;
    missing_status_completed().await;
    null_status_completed().await;
    empty_status_rejected().await;
    unknown_status_rejected().await;
    message_data_unfiltered().await;
    extras_kept().await;
    thread_id_quoted().await;
    empty_thread_id_refused().await;
    dot_segment_refused().await;
}

// The thread id travels as the first positional argument and every message_data field reaches the body.
async fn status_preserved() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE_IN_PROGRESS);
    let reply = api
        .call(
            &request,
            None,
            None,
            MessageCall::Append {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "POST",
            MESSAGES_URL,
            Some(MESSAGE_DATA),
            Auth::Key(KEY)
        )]
    );
    assert_eq!(reply.kind, ClientKind::Sync);
    let MessageReply::Message(message) = reply.reply else {
        panic!("append returns a message");
    };
    assert_eq!(*message, expected(MESSAGE_IN_PROGRESS));
}

// LiteLLM fills a status the SDK left out with completed before validating the message.
async fn missing_status_completed() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE_NO_STATUS);
    let reply = api
        .call(
            &request,
            None,
            None,
            MessageCall::Append {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 1);
    let MessageReply::Message(message) = reply.reply else {
        panic!("append returns a message");
    };
    assert_eq!(*message, expected(MESSAGE));
    assert_eq!(message.status, Some(MessageStatus::Completed));
}

async fn null_status_completed() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE_NULL_STATUS);
    let reply = api
        .call(
            &request,
            None,
            None,
            MessageCall::Append {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 1);
    let MessageReply::Message(message) = reply.reply else {
        panic!("append returns a message");
    };
    assert_eq!(*message, expected(MESSAGE));
}

// An empty or unknown status stays as the SDK wrote it and the OpenAIMessage validation fails.
async fn empty_status_rejected() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE_EMPTY_STATUS);
    let error = api
        .call(
            &request,
            None,
            None,
            MessageCall::Append {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MessagesError::Threads(ThreadsError::Reply {
            fault: ReplyFault::Shape,
            ..
        })
    ));
    assert_eq!(transport.requests().len(), 1);
}

async fn unknown_status_rejected() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE_UNKNOWN_STATUS);
    let error = api
        .call(
            &request,
            None,
            None,
            MessageCall::Append {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MessagesError::Threads(ThreadsError::Reply {
            fault: ReplyFault::Shape,
            ..
        })
    ));
    assert_eq!(transport.requests().len(), 1);
}

// Append hands message_data to the SDK itself; the legacy attachment mapper runs nowhere on this path.
async fn message_data_unfiltered() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_ATTACHMENTS);
    let (api, transport) = harness(200, MESSAGE);
    api.call(
        &request,
        None,
        None,
        MessageCall::Append {
            thread_id: "thread_1",
            data: &data,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "POST",
            MESSAGES_URL,
            Some(MESSAGE_ATTACHMENTS),
            Auth::Key(KEY)
        )]
    );
}

async fn extras_kept() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE_EXTRA);
    let reply = api
        .call(
            &request,
            None,
            None,
            MessageCall::Append {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 1);
    let MessageReply::Message(message) = reply.reply else {
        panic!("append returns a message");
    };
    assert_eq!(*message, expected(MESSAGE_EXTRA));
}

async fn thread_id_quoted() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE);
    api.call(
        &request,
        None,
        None,
        MessageCall::Append {
            thread_id: "thread a/b?c",
            data: &data,
        },
    )
    .await
    .unwrap();
    assert_eq!(transport.requests()[0].url, QUOTED_URL);
}

async fn empty_thread_id_refused() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE);
    let error = api
        .call(
            &request,
            None,
            None,
            MessageCall::Append {
                thread_id: "",
                data: &data,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MessagesError::Threads(ThreadsError::EmptyThreadId)
    ));
    assert!(transport.requests().is_empty());
}

async fn dot_segment_refused() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE);
    let error = api
        .call(
            &request,
            None,
            None,
            MessageCall::Append {
                thread_id: "..",
                data: &data,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MessagesError::Threads(ThreadsError::DotSegment { .. })
    ));
    assert!(transport.requests().is_empty());
}
