#![cfg(test)]

use betterlite_http::client_kind::ClientKind;
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
    status_outcomes().await;
    message_data_unfiltered().await;
    extras_kept().await;
    thread_id_quoted().await;
    invalid_thread_id_refused().await;
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

// An absent or null status is filled with completed before validation; an empty or unknown one fails it.
async fn status_outcomes() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    for (body, completes) in [
        (MESSAGE_NO_STATUS, true),
        (MESSAGE_NULL_STATUS, true),
        (MESSAGE_EMPTY_STATUS, false),
        (MESSAGE_UNKNOWN_STATUS, false),
    ] {
        let (api, transport) = harness(200, body);
        let call = MessageCall::Append {
            thread_id: "thread_1",
            data: &data,
        };
        let outcome = api.call(&request, None, None, call).await;
        assert_eq!(transport.requests().len(), 1);
        if completes {
            let MessageReply::Message(message) =
                outcome.expect("an absent status is completed").reply
            else {
                panic!("append returns a message");
            };
            assert_eq!(message.status, Some(MessageStatus::Completed));
            assert_eq!(*message, expected(MESSAGE));
        } else {
            assert!(matches!(
                outcome.unwrap_err(),
                MessagesError::Threads(ThreadsError::Reply {
                    fault: ReplyFault::Shape,
                    ..
                })
            ));
        }
    }
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

async fn invalid_thread_id_refused() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let empty_call = MessageCall::Append {
        thread_id: "",
        data: &data,
    };
    let (empty_api, empty_transport) = harness(200, MESSAGE);
    let empty = empty_api
        .call(&request, None, None, empty_call)
        .await
        .unwrap_err();
    assert!(matches!(
        empty,
        MessagesError::Threads(ThreadsError::EmptyThreadId)
    ));
    assert!(empty_transport.requests().is_empty());
    let dot_call = MessageCall::Append {
        thread_id: "..",
        data: &data,
    };
    let (dot_api, dot_transport) = harness(200, MESSAGE);
    let dot = dot_api
        .call(&request, None, None, dot_call)
        .await
        .unwrap_err();
    assert!(matches!(
        dot,
        MessagesError::Threads(ThreadsError::DotSegment { .. })
    ));
    assert!(dot_transport.requests().is_empty());
}
