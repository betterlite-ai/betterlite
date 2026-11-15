use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;

use crate::client::records::MessageCommon;
use crate::client::records::compat::{Message, MessageKind, Part, Role, TextPart};
use crate::client::records::response::{SendMessageResponse, StreamResult};
use crate::completion_bridge::tests::support::parsed;
use crate::send::error::SendCallError;
use crate::send::response::{response_from_a2a, response_from_dict};

use super::support::request;

fn text_of(result: &BTreeMap<String, AgentJson>) -> Option<&str> {
    result
        .get("parts")
        .and_then(AgentJson::as_array)
        .and_then(|parts| parts.first())
        .and_then(|part| part.get("text"))
        .and_then(AgentJson::as_str)
}

#[test]
fn send_response_wraps_the_native_answer() {
    let response = SendMessageResponse {
        jsonrpc: "2.0".to_owned(),
        id: JsonRpcId::String("rpc-1".to_owned()),
        result: StreamResult::Message(Message {
            kind: MessageKind::Message,
            role: Role::Agent,
            parts: vec![Part::Text(TextPart {
                text: "hi".to_owned(),
                metadata: None,
            })],
            common: MessageCommon {
                message_id: "m-1".to_owned(),
                context_id: None,
                task_id: None,
                reference_task_ids: None,
                metadata: None,
                extensions: None,
            },
        }),
    };

    let wrapped = response_from_a2a(&response, &request("hello")).expect("the response wraps");

    assert_eq!(wrapped.id, Some(JsonRpcId::String("rpc-1".to_owned())));
    assert_eq!(wrapped.jsonrpc.0, "2.0");
    assert!(wrapped.error.is_none());
    assert!(wrapped.usage.is_none());
    assert!(wrapped.hidden_params.is_empty());
    let result = wrapped.result.expect("the message result is a map");
    assert_eq!(text_of(&result), Some("hi"));
}

#[test]
fn send_response_backfills_a_missing_request_id() {
    let answer = parsed(
        r#"{"jsonrpc":"2.0","result":{"kind":"message","role":"agent","parts":[],"messageId":"m-1"}}"#,
    );

    let wrapped = response_from_dict(&answer, "request-1").expect("the bridge answer wraps");

    assert_eq!(wrapped.id, Some(JsonRpcId::String("request-1".to_owned())));
    assert!(wrapped.result.is_some());
}

#[test]
fn send_response_keeps_an_error_payload_and_its_bool_id() {
    let answer =
        parsed(r#"{"jsonrpc":"2.0","id":true,"error":{"code":-32601,"message":"no method"}}"#);

    let wrapped = response_from_dict(&answer, "request-1").expect("the error payload wraps");

    assert_eq!(wrapped.id, Some(JsonRpcId::String("True".to_owned())));
    assert!(wrapped.result.is_none());
    let error = wrapped.error.expect("the error is kept");
    assert_eq!(error.get("code"), Some(&parsed("-32601")));
    assert_eq!(error.get("message"), Some(&parsed(r#""no method""#)));
}

#[test]
fn send_response_rejects_a_non_object_answer() {
    let result = response_from_dict(&parsed("[1, 2]"), "request-1");

    assert!(matches!(result, Err(SendCallError::Response(_))));
}
