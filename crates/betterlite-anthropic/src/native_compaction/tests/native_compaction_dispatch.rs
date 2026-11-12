#![cfg(test)]

use rstest::rstest;

use crate::native_compaction::dispatch::dispatch;
use crate::native_compaction::dispatch::error::DispatchError;
use crate::native_compaction::error::NativeCompactionError;
use crate::native_compaction::protocol::CompactionProtocol;

use super::fixtures::{FakeFailure, FakeRouter, mapping};

const PAYLOAD: &str = r#"{"model":"claude-opus-4-7","messages":[{"role":"user","content":"hi"}],"stream":true,"max_tokens":0,"temperature":null,"compaction":{"type":"summarize"}}"#;

#[tokio::test]
async fn native_compaction_dispatch_messages_passes_the_payload_through() {
    let router = FakeRouter::replying(Ok(mapping(r#"{"id":"m"}"#)));
    let payload = mapping(PAYLOAD);

    let reply = dispatch(&router, CompactionProtocol::Messages, &payload)
        .await
        .unwrap();

    assert_eq!(reply, mapping(r#"{"id":"m"}"#));
    assert_eq!(router.messages_recorded(), vec![payload]);
    assert!(router.chat_recorded().is_empty());
}

#[tokio::test]
async fn native_compaction_dispatch_chat_splits_model_messages_and_options() {
    let router = FakeRouter::replying(Ok(mapping(r#"{"id":"c"}"#)));
    let payload = mapping(PAYLOAD);

    let reply = dispatch(&router, CompactionProtocol::Chat, &payload)
        .await
        .unwrap();

    assert_eq!(reply, mapping(r#"{"id":"c"}"#));
    assert!(router.messages_recorded().is_empty());
    let calls = router.chat_recorded();
    let [call] = calls.as_slice() else {
        panic!("expected one chat call, got {}", calls.len());
    };
    assert_eq!(call.model, "claude-opus-4-7");
    assert_eq!(call.messages.len(), 1);
    assert!(!call.stream);
    let keys: Vec<&str> = call.options.keys().map(String::as_str).collect();
    assert_eq!(keys, ["max_tokens", "temperature", "compaction"]);
}

#[rstest]
#[case::no_model(r#"{"messages":[]}"#, NativeCompactionError::Missing { field: "model" })]
#[case::numeric_model(r#"{"model":1,"messages":[]}"#, NativeCompactionError::NotText { field: "model" })]
#[case::no_messages(r#"{"model":"m"}"#, NativeCompactionError::Missing { field: "messages" })]
#[case::null_messages(r#"{"model":"m","messages":null}"#, NativeCompactionError::NotAList { field: "messages" })]
#[case::object_messages(r#"{"model":"m","messages":{}}"#, NativeCompactionError::NotAList { field: "messages" })]
#[case::text_message(r#"{"model":"m","messages":["hi"]}"#, NativeCompactionError::BadMessage { index: 0 })]
#[case::message_without_role(r#"{"model":"m","messages":[{"role":"user"},{"content":"x"}]}"#, NativeCompactionError::BadMessage { index: 1 })]
#[case::unknown_role(r#"{"model":"m","messages":[{"role":"critic"}]}"#, NativeCompactionError::BadMessage { index: 0 })]
#[case::numeric_role(r#"{"model":"m","messages":[{"role":1}]}"#, NativeCompactionError::BadMessage { index: 0 })]
#[tokio::test]
async fn native_compaction_dispatch_chat_rejects_a_bad_payload(
    #[case] payload: &str,
    #[case] expected: NativeCompactionError,
) {
    let router = FakeRouter::replying(Ok(mapping("{}")));

    let error = dispatch(&router, CompactionProtocol::Chat, &mapping(payload))
        .await
        .unwrap_err();

    assert!(
        matches!(&error, DispatchError::Payload { source } if source.to_string() == expected.to_string())
    );
    assert!(router.chat_recorded().is_empty());
}

#[rstest]
#[case::user("user")]
#[case::assistant("assistant")]
#[case::tool("tool")]
#[case::system("system")]
#[case::function("function")]
#[case::developer("developer")]
#[tokio::test]
async fn native_compaction_dispatch_chat_accepts_every_message_role(#[case] role: &str) {
    let router = FakeRouter::replying(Ok(mapping("{}")));
    let payload = mapping(&format!(
        r#"{{"model":"m","messages":[{{"role":"{role}"}}]}}"#
    ));

    dispatch(&router, CompactionProtocol::Chat, &payload)
        .await
        .unwrap();

    assert_eq!(router.chat_recorded().len(), 1);
}

#[rstest]
#[case::messages(CompactionProtocol::Messages)]
#[case::chat(CompactionProtocol::Chat)]
#[tokio::test]
async fn native_compaction_dispatch_passes_router_errors_up(#[case] protocol: CompactionProtocol) {
    let router = FakeRouter::replying(Err(FakeFailure));

    let error = dispatch(&router, protocol, &mapping(PAYLOAD))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        DispatchError::Router {
            source: FakeFailure
        }
    ));
}
