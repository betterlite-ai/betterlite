#![cfg(test)]

use std::sync::Arc;

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::threads::records::ThreadMessagePage;

use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_assistant_definitions::operations::AssistantsApi;
use crate::azure_message_attachments::error::MessagesError;
use crate::azure_message_attachments::operations::{MessageCall, MessageReply, MessagesApi};

use super::support::{
    Auth, ERROR_BODY, Fixture, KEY, MESSAGES_URL, PAGE, PAGE_WITH_MESSAGE, Recorder, harness, wire,
};

const PAGE_MISSING_STATUS: &str = r#"{"object":"list","data":[{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"user","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}],"first_id":"msg_1","last_id":"msg_1","has_more":false}"#;

fn expected(json: &str) -> ThreadMessagePage {
    serde_json::from_str(json).unwrap()
}

pub(super) async fn check() {
    no_query().await;
    page_unchanged().await;
    missing_status_kept().await;
    async_flag().await;
    http_status_propagates().await;
    transport_failure_propagates().await;
}

// LiteLLM calls list(thread_id=...) with no order, limit, before or after.
async fn no_query() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let reply = api
        .call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire("GET", MESSAGES_URL, None, Auth::Key(KEY))]
    );
    assert_eq!(reply.kind, ClientKind::Sync);
    let MessageReply::Page(page) = reply.reply else {
        panic!("list returns a page");
    };
    assert_eq!(*page, expected(PAGE));
}

// The cursor page comes back exactly as the SDK built it, extra fields included.
async fn page_unchanged() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE_WITH_MESSAGE);
    let reply = api
        .call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 1);
    let MessageReply::Page(page) = reply.reply else {
        panic!("list returns a page");
    };
    assert_eq!(*page, expected(PAGE_WITH_MESSAGE));
}

// Nothing fills in a status on the list path, so an absent one stays absent.
async fn missing_status_kept() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE_MISSING_STATUS);
    let reply = api
        .call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 1);
    let MessageReply::Page(page) = reply.reply else {
        panic!("list returns a page");
    };
    assert_eq!(*page, expected(PAGE_MISSING_STATUS));
    assert_eq!(page.data[0].status, None);
}

// Only the literal true flag selects the async client; a missing or false one stays sync.
async fn async_flag() {
    async_kind(Some(true), ClientKind::Asynchronous).await;
    async_kind(Some(false), ClientKind::Sync).await;
    async_kind(None, ClientKind::Sync).await;
}

async fn async_kind(flag: Option<bool>, kind: ClientKind) {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let reply = api
        .call(&request, None, flag, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 1);
    assert_eq!(reply.kind, kind);
}

// The assistant client's status and transport failures surface unchanged inside the facade error.
async fn http_status_propagates() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(404, ERROR_BODY);
    let error = api
        .call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MessagesError::Definitions(DefinitionsError::Assistants(AssistantsError::Status {
            status: 404,
            ..
        }))
    ));
    assert_eq!(transport.requests().len(), 1);
}

async fn transport_failure_propagates() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let transport = Arc::new(Recorder::default());
    transport.fail_next();
    let api = MessagesApi {
        assistants: AssistantsApi {
            transport: Arc::clone(&transport),
        },
    };
    let error = api
        .call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MessagesError::Definitions(DefinitionsError::Assistants(
            AssistantsError::Transport { .. }
        ))
    ));
    assert_eq!(
        transport.requests(),
        vec![wire("GET", MESSAGES_URL, None, Auth::Key(KEY))]
    );
}
