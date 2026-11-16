#![cfg(test)]

use std::sync::Arc;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::operations::ThreadReply;
use betterlite_openai::threads::records::{MessageStatus, ThreadCreate, ThreadMessageData};

use crate::openai_assistant_dispatch::OpenAiAssistantResult;
use crate::openai_assistant_dispatch::call::{
    OpenAiAssistantOperation, ThreadsCall, ThreadsOperation,
};
use crate::openai_assistant_dispatch::error::OpenAiAssistantError;

use super::support::{BASE, Recorder, RecordingLogger, options, run_operation};

const THREAD: &str =
    r#"{"id":"thread_1","object":"thread","created_at":7,"metadata":{"team":"core"}}"#;
const MESSAGE: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}"#;
const MESSAGE_PAGE: &str = r#"{"object":"list","data":[{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}],"first_id":"msg_1","last_id":"msg_1","has_more":false}"#;

fn thread_data() -> ThreadCreate {
    serde_json::from_str(
        r#"{"metadata":{"team":"core"},"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .unwrap()
}

fn threads_operation(operation: ThreadsOperation) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Threads(ThreadsCall {
        options: options(),
        operation,
    })
}

async fn threads_create() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, THREAD);
    let mut logger = RecordingLogger::default();
    let call = threads_operation(ThreadsOperation::Create {
        data: thread_data(),
        async_requested: true,
    });
    let created = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Threads(reply) = created else {
        panic!("create did not return a thread")
    };
    let ThreadReply::Thread(thread) = *reply else {
        panic!()
    };
    assert_eq!(thread.id, "thread_1");
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads"));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
    let expected: JsonValue = serde_json::from_str(
        r#"{"metadata":{"team":"core"},"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .unwrap();
    assert_eq!(seen[0].body.as_ref(), Some(&expected));
}

async fn threads_retrieve() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, THREAD);
    let mut logger = RecordingLogger::default();
    let call = threads_operation(ThreadsOperation::Retrieve {
        thread_id: String::from("thread_1"),
        async_requested: false,
    });
    let retrieved = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Threads(reply) = retrieved else {
        panic!("retrieve did not return a thread")
    };
    let ThreadReply::Thread(thread) = *reply else {
        panic!()
    };
    assert_eq!(thread.id, "thread_1");
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_1"));
    assert_eq!(seen[0].body, None);
}

async fn threads_add_message() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, MESSAGE);
    let mut logger = RecordingLogger::default();
    let call = threads_operation(ThreadsOperation::AddMessage {
        thread_id: String::from("thread_1"),
        data: Box::new(
            serde_json::from_str::<ThreadMessageData>(r#"{"role":"user","content":"hi"}"#).unwrap(),
        ),
        async_requested: true,
    });
    let added = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Threads(reply) = added else {
        panic!("add_message did not return a message")
    };
    let ThreadReply::Message(message) = *reply else {
        panic!()
    };
    assert_eq!(message.id, "msg_1");
    assert_eq!(message.status, Some(MessageStatus::Completed));
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_1/messages"));
    let expected: JsonValue = serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap();
    assert_eq!(seen[0].body.as_ref(), Some(&expected));
}

async fn threads_list_messages() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, MESSAGE_PAGE);
    let mut logger = RecordingLogger::default();
    let call = threads_operation(ThreadsOperation::ListMessages {
        thread_id: String::from("thread_1"),
        async_requested: false,
    });
    let listed = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Threads(reply) = listed else {
        panic!("list_messages did not return a page")
    };
    let ThreadReply::Page(page) = *reply else {
        panic!()
    };
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id, "msg_1");
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_1/messages"));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
}

async fn threads_missing_key() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = OpenAiAssistantOperation::Threads(ThreadsCall {
        options: ClientOptions::default(),
        operation: ThreadsOperation::Retrieve {
            thread_id: String::from("thread_1"),
            async_requested: false,
        },
    });
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the missing api key did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Threads(ThreadsError::MissingApiKey)
    ));
}

pub(super) async fn case() {
    threads_create().await;
    threads_retrieve().await;
    threads_add_message().await;
    threads_list_messages().await;
    threads_missing_key().await;
}
