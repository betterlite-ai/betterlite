use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use crate::agent_endpoints::a2a_stream::framing::{
    INTERNAL_ERROR, INVALID_PARAMS, convert_event, error_event, to_jsonrpc_object,
};
use crate::agent_endpoints::a2a_stream::tests::fakes::{CountingConverter, json};

#[rstest]
#[case::object_text(
    r#""{\"jsonrpc\": \"2.0\", \"id\": 1}""#,
    r#"{"jsonrpc": "2.0", "id": 1}"#
)]
#[case::list_text(r#""[1, 2]""#, "[1, 2]")]
#[case::null_text(r#""null""#, "null")]
#[case::unparsable_text(r#""plain text""#, r#""plain text""#)]
#[case::empty_text(r#""""#, r#""""#)]
#[case::object(r#"{"result": "{}"}"#, r#"{"result": "{}"}"#)]
#[case::number("5", "5")]
#[case::null("null", "null")]
fn to_jsonrpc_object_parses_only_string_chunks(#[case] chunk: &str, #[case] expected: &str) {
    assert_eq!(to_jsonrpc_object(json(chunk)), json(expected));
}

#[rstest]
#[case::text_id(
    r#""r1""#,
    INTERNAL_ERROR,
    r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": "boom"}}"#
)]
#[case::number_id(
    "7",
    INVALID_PARAMS,
    r#"{"jsonrpc": "2.0", "id": 7, "error": {"code": -32602, "message": "boom"}}"#
)]
#[case::null_id(
    "null",
    INTERNAL_ERROR,
    r#"{"jsonrpc": "2.0", "id": null, "error": {"code": -32603, "message": "boom"}}"#
)]
fn error_event_builds_a_jsonrpc_error(
    #[case] request_id: &str,
    #[case] code: i64,
    #[case] expected: &str,
) {
    assert_eq!(error_event(&json(request_id), code, "boom"), json(expected));
}

#[rstest]
#[case::object(
    r#"{"kind": "task"}"#,
    r#"{"kind": "task", "conversions": 1, "request_id": "r1"}"#
)]
#[case::list(r#"[{"kind": "task"}]"#, r#"[{"kind": "task"}]"#)]
#[case::text(r#""{}""#, r#""{}""#)]
#[case::number("3", "3")]
fn convert_event_converts_only_objects(#[case] event: &str, #[case] expected: &str) {
    let request_id = AgentJson::from("r1");
    assert_eq!(
        convert_event(&CountingConverter, json(event), Some(&request_id)),
        json(expected)
    );
}
