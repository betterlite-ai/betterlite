use betterlite_openai::threads::records::MessageStatus;

use super::connection::MessageParams;
use super::error::{MessageError, SetupError};
use super::service::Operation;
use super::{Fixture, MESSAGE_REPLY, call, data, json, message_of, page_of, params, request};

const PAGE_REPLY: &str = r#"{"object":"list","data":[],"has_more":false}"#;

#[test]
fn append_openai_message() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(
        r#"{"role":"user","content":"hello","attachments":[{"file_id":"file_a","tools":[{"type":"file_search"}]}],"metadata":{"tag":"x"}}"#,
    );
    let replied = message_of(
        call(
            &fixture,
            &request("openai", Some(&message), &params("openai")),
            Operation::Append,
            false,
        )
        .unwrap(),
    );
    assert_eq!(replied.id, "msg_a");
    assert_eq!(replied.thread_id, "thread_a");
    assert_eq!(replied.status, Some(MessageStatus::Completed));
    let seen = fixture.transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        "https://openai.test/v1/threads/thread_a/messages"
    );
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(
        seen[0].headers.get("openai-beta").map(String::as_str),
        Some("assistants=v2")
    );
    assert_eq!(
        seen[0].body,
        Some(json(
            r#"{"role":"user","content":"hello","attachments":[{"file_id":"file_a","tools":[{"type":"file_search"}]}],"metadata":{"tag":"x"}}"#
        ))
    );
}

#[test]
fn append_openai_message_async_repeats_the_request() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let sync = message_of(
        call(
            &fixture,
            &request("openai", Some(&message), &params("openai")),
            Operation::Append,
            false,
        )
        .unwrap(),
    );
    let asynchronous = message_of(
        call(
            &fixture,
            &request("openai", Some(&message), &params("openai")),
            Operation::Append,
            true,
        )
        .unwrap(),
    );
    assert_eq!(sync.id, asynchronous.id);
    let seen = fixture.transport.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].body, seen[1].body);
    assert_eq!(
        seen[0].body,
        Some(json(r#"{"role":"user","content":"hello"}"#))
    );
}

#[test]
fn append_openai_message_keeps_empty_lists_and_maps() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello","attachments":[],"metadata":{}}"#);
    call(
        &fixture,
        &request("openai", Some(&message), &params("openai")),
        Operation::Append,
        false,
    )
    .unwrap();
    assert_eq!(
        fixture.transport.requests()[0].body,
        Some(json(
            r#"{"role":"user","content":"hello","attachments":[],"metadata":{}}"#
        ))
    );
}

#[test]
fn append_openai_message_passes_content_blocks_unchanged() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":[{"type":"text","text":"hello"}]}"#);
    call(
        &fixture,
        &request("openai", Some(&message), &params("openai")),
        Operation::Append,
        false,
    )
    .unwrap();
    assert_eq!(
        fixture.transport.requests()[0].body,
        Some(json(
            r#"{"role":"user","content":[{"type":"text","text":"hello"}]}"#
        ))
    );
}

#[test]
fn append_openai_message_rejects_a_system_role_at_the_typed_boundary() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"system","content":"hello"}"#);
    let error = call(
        &fixture,
        &request("openai", Some(&message), &params("openai")),
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert!(matches!(
        error.error,
        MessageError::Setup(SetupError::Data { .. })
    ));
    assert!(fixture.transport.requests().is_empty());
}

#[test]
fn message_list_ignores_pagination() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, PAGE_REPLY);
    fixture.transport.queue(200, PAGE_REPLY);
    let params: MessageParams = serde_json::from_str(
        r#"{"after":"msg_a","before":"msg_z","limit":2,"order":"asc","a_add_message":1,"aget_messages":"true","api_base":"https://openai.test/v1","api_key":"sk-test"}"#,
    )
    .unwrap();
    let sync_page = page_of(
        call(
            &fixture,
            &request("openai", None, &params),
            Operation::List,
            false,
        )
        .unwrap(),
    );
    let async_page = page_of(
        call(
            &fixture,
            &request("openai", None, &params),
            Operation::List,
            true,
        )
        .unwrap(),
    );
    assert!(sync_page.data.is_empty());
    assert_eq!(sync_page.has_more, Some(false));
    assert_eq!(sync_page.extra.get("object"), Some(&json(r#""list""#)));
    assert_eq!(async_page, sync_page);
    let seen = fixture.transport.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(
        seen[0].url,
        "https://openai.test/v1/threads/thread_a/messages"
    );
    assert!(seen[0].body.is_none());
    assert_eq!(seen[1].method, "GET");
    assert_eq!(
        seen[1].url,
        "https://openai.test/v1/threads/thread_a/messages"
    );
    assert!(seen[1].body.is_none());
}
