#![cfg(test)]

mod a2a_stream_tests_behavior {
    use betterlite_core::agents::json::AgentJson;
    use rstest::rstest;
    use serde_json::json;

    use crate::agent_endpoints::a2a_stream::framing::{
        INTERNAL_ERROR, INVALID_PARAMS, convert_event, error_event, to_jsonrpc_object,
    };
    use crate::agent_endpoints::a2a_stream::tests::fakes::CountingConverter;

    #[rstest]
    #[case::object_text(json!("{\"jsonrpc\": \"2.0\", \"id\": 1}"), json!({"jsonrpc": "2.0", "id": 1_i32}))]
    #[case::list_text(json!("[1, 2]"), json!([1_i32, 2_i32]))]
    #[case::null_text(json!("null"), AgentJson::Null)]
    #[case::unparsable_text(json!("plain text"), json!("plain text"))]
    #[case::empty_text(json!(""), json!(""))]
    #[case::object(json!({"result": "{}"}), json!({"result": "{}"}))]
    #[case::number(json!(5_i32), json!(5_i32))]
    #[case::null(AgentJson::Null, AgentJson::Null)]
    fn to_jsonrpc_object_parses_only_string_chunks(
        #[case] chunk: AgentJson,
        #[case] expected: AgentJson,
    ) {
        assert_eq!(to_jsonrpc_object(chunk), expected);
    }

    #[rstest]
    #[case::text_id(json!("r1"), INTERNAL_ERROR, json!({"jsonrpc": "2.0", "id": "r1", "error": {"code": -32_603_i64, "message": "boom"}}))]
    #[case::number_id(json!(7_i32), INVALID_PARAMS, json!({"jsonrpc": "2.0", "id": 7_i32, "error": {"code": -32_602_i64, "message": "boom"}}))]
    #[case::null_id(AgentJson::Null, INTERNAL_ERROR, json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32_603_i64, "message": "boom"}}))]
    fn error_event_builds_a_jsonrpc_error(
        #[case] request_id: AgentJson,
        #[case] code: i64,
        #[case] expected: AgentJson,
    ) {
        assert_eq!(error_event(&request_id, code, "boom"), expected);
    }

    #[rstest]
    #[case::object(json!({"kind": "task"}), json!({"kind": "task", "conversions": 1_i32, "request_id": "r1"}))]
    #[case::list(json!([{"kind": "task"}]), json!([{"kind": "task"}]))]
    #[case::text(json!("{}"), json!("{}"))]
    #[case::number(json!(3_i32), json!(3_i32))]
    fn convert_event_converts_only_objects(#[case] event: AgentJson, #[case] expected: AgentJson) {
        let request_id = json!("r1");
        assert_eq!(
            convert_event(&CountingConverter, event, Some(&request_id)),
            expected
        );
    }
}
