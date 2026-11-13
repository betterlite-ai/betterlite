use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::singulr::SingulrLoggingObject;
use crate::singulr::is_mcp_call;
use crate::singulr::payload;

use super::support;

fn sent(
    inputs: &str,
    request_data: &str,
    input_type: GuardrailInputType,
    mcp: bool,
) -> crate::singulr::payload::SingulrPayload {
    payload::build(
        &support::inputs(inputs),
        &support::data(request_data),
        input_type,
        mcp,
    )
    .unwrap()
}

#[test]
fn a_chat_request_sends_the_structured_messages_tools_and_images() {
    let payload = sent(
        r#"{"texts": ["hello"], "structured_messages": [{"role": "user", "content": "hello"}],
            "images": ["img-1"], "tools": [{"type": "function", "function": {"name": "f"}}],
            "model": "gpt-4o"}"#,
        r#"{"litellm_call_id": "call-1", "metadata": {"user_api_key_alias": "alias"}}"#,
        GuardrailInputType::Request,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": "call-1", "model_name": "gpt-4o", "model_provider_name": null,
                "guardrail_scope": "request",
                "messages": [{"role": "user", "content": "hello"}],
                "images": ["img-1"], "tools": [{"type": "function", "function": {"name": "f"}}],
                "response": null, "metadata": {"user_api_key_alias": "alias"}}"#
        )
    );
}

#[test]
fn a_chat_request_builds_one_user_message_per_text() {
    let payload = sent(
        r#"{"texts": ["first", "second"]}"#,
        "{}",
        GuardrailInputType::Request,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "request",
                "messages": [{"role": "user", "content": "first"}, {"role": "user", "content": "second"}],
                "images": null, "tools": null, "response": null, "metadata": null}"#
        )
    );
}

#[test]
fn a_chat_request_with_nothing_to_scan_sends_no_payload() {
    let built = payload::build(
        &support::inputs(r#"{"texts": [], "structured_messages": [], "images": [], "tools": []}"#),
        &support::data("{}"),
        GuardrailInputType::Request,
        false,
    );
    assert!(built.is_none());
}

#[test]
fn images_alone_still_build_a_chat_request() {
    let payload = sent(
        r#"{"texts": [], "images": ["img-1"]}"#,
        "{}",
        GuardrailInputType::Request,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "request", "messages": [], "images": ["img-1"],
                "tools": null, "response": null, "metadata": null}"#
        )
    );
}

#[test]
fn a_chat_response_joins_the_texts_and_converts_the_tool_calls() {
    let payload = sent(
        r#"{"texts": ["first", "second"], "model": "gpt-4o",
            "tool_calls": [{"id": "call-1", "type": "function",
                            "function": {"name": "weather", "arguments": "{\"city\": \"Paris\"}"}}]}"#,
        r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "q"}]}"#,
        GuardrailInputType::Response,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": "gpt-4o", "model_provider_name": null,
                "guardrail_scope": "response", "messages": [{"role": "user", "content": "q"}],
                "images": null, "tools": null,
                "response": {"role": "assistant", "content": "first\nsecond",
                             "tool_calls": [{"id": "call-1", "type": "function",
                                             "function": {"name": "weather", "arguments": "{\"city\": \"Paris\"}"}}]},
                "metadata": null}"#
        )
    );
}

#[test]
fn a_chat_response_without_texts_sends_null_content() {
    let payload = sent(
        r#"{"texts": []}"#,
        "{}",
        GuardrailInputType::Response,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "response", "messages": null, "images": null, "tools": null,
                "response": {"role": "assistant", "content": null, "tool_calls": []},
                "metadata": null}"#
        )
    );
}

#[test]
fn an_mcp_request_sends_the_tool_name_and_arguments() {
    let payload = sent(
        r#"{"texts": ["x"]}"#,
        r#"{"mcp_tool_name": "read_file", "mcp_arguments": {"path": "/tmp/a"},
            "mcp_server_name": "files", "name": "ignored"}"#,
        GuardrailInputType::Request,
        true,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"model_name": null, "guardrail_scope": "mcp_request", "tool_name": "read_file",
                "tool_arguments": {"path": "/tmp/a"}, "mcp_server_name": "files",
                "tool_result": null, "metadata": null}"#
        )
    );
}

#[test]
fn a_null_mcp_arguments_falls_back_to_arguments() {
    let payload = sent(
        r#"{"texts": ["x"]}"#,
        r#"{"mcp_tool_name": "t", "mcp_arguments": null, "arguments": {"a": 1}}"#,
        GuardrailInputType::Request,
        true,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"model_name": null, "guardrail_scope": "mcp_request", "tool_name": "t",
                "tool_arguments": {"a": 1}, "mcp_server_name": null, "tool_result": null,
                "metadata": null}"#
        )
    );
}

#[test]
fn an_mcp_request_names_the_tool_from_the_name_field_when_the_tool_name_is_absent() {
    let payload = sent(
        r#"{"texts": ["x"]}"#,
        r#"{"name": "from-name"}"#,
        GuardrailInputType::Request,
        true,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"model_name": null, "guardrail_scope": "mcp_request", "tool_name": "from-name",
                "tool_arguments": null, "mcp_server_name": null, "tool_result": null,
                "metadata": null}"#
        )
    );
}

#[test]
fn an_mcp_response_sends_the_texts_as_the_tool_result() {
    let payload = sent(
        r#"{"texts": ["a", "b"]}"#,
        r#"{"model": "MCP: files"}"#,
        GuardrailInputType::Response,
        true,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"model_name": "MCP: files", "guardrail_scope": "mcp_response", "tool_name": null,
                "tool_arguments": null, "mcp_server_name": null, "tool_result": ["a", "b"],
                "metadata": null}"#
        )
    );
}

#[test]
fn an_mcp_response_without_texts_sends_no_payload() {
    let built = payload::build(
        &support::inputs(r#"{"texts": []}"#),
        &support::data(r#"{"model": "MCP: files"}"#),
        GuardrailInputType::Response,
        true,
    );
    assert!(built.is_none());
}

#[test]
fn the_metadata_takes_the_first_container_in_order() {
    let payload = sent(
        r#"{"texts": ["x"]}"#,
        r#"{"litellm_metadata": {"user_api_key_alias": "from-litellm"},
            "metadata": {"user_api_key_alias": "from-metadata", "user_api_key_user_id": "u1"},
            "litellm_params": {"litellm_metadata": {"user_api_key_user_id": "u2"},
                               "metadata": {"user_api_key_user_email": "e@example.com"}}}"#,
        GuardrailInputType::Request,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "request", "messages": [{"role": "user", "content": "x"}],
                "images": null, "tools": null, "response": null,
                "metadata": {"user_api_key_alias": "from-litellm", "user_api_key_user_id": "u1",
                             "user_api_key_user_email": "e@example.com"}}"#
        )
    );
}

#[test]
fn an_empty_metadata_container_is_skipped() {
    let payload = sent(
        r#"{"texts": ["x"]}"#,
        r#"{"litellm_metadata": {}, "metadata": {"user_api_key_alias": "second"}}"#,
        GuardrailInputType::Request,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "request", "messages": [{"role": "user", "content": "x"}],
                "images": null, "tools": null, "response": null,
                "metadata": {"user_api_key_alias": "second"}}"#
        )
    );
}

#[test]
fn an_empty_metadata_value_falls_through_to_the_next_container() {
    let payload = sent(
        r#"{"texts": ["x"]}"#,
        r#"{"metadata": {"user_api_key_alias": "", "user_api_key_user_id": null},
            "litellm_params": {"metadata": {"user_api_key_alias": "later"}}}"#,
        GuardrailInputType::Request,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "request", "messages": [{"role": "user", "content": "x"}],
                "images": null, "tools": null, "response": null,
                "metadata": {"user_api_key_alias": "later"}}"#
        )
    );
}

#[test]
fn the_user_role_comes_from_the_first_key_auth_object_that_has_one() {
    let payload = sent(
        r#"{"texts": ["x"]}"#,
        r#"{"metadata": {"user_api_key_auth": {"user_role": ""}},
            "litellm_params": {"metadata": {"user_api_key_auth": {"user_role": "proxy_admin"}}}}"#,
        GuardrailInputType::Request,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "request", "messages": [{"role": "user", "content": "x"}],
                "images": null, "tools": null, "response": null,
                "metadata": {"user_api_key_user_role": "proxy_admin"}}"#
        )
    );
}

#[test]
fn no_metadata_field_keeps_the_metadata_null() {
    let payload = sent(
        r#"{"texts": ["x"]}"#,
        r#"{"metadata": {"user_api_key_alias": ""}, "litellm_params": "not-an-object"}"#,
        GuardrailInputType::Request,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "request", "messages": [{"role": "user", "content": "x"}],
                "images": null, "tools": null, "response": null, "metadata": null}"#
        )
    );
}

#[rstest]
#[case::missing_id(r#"{"function": {"name": "f", "arguments": "{}"}}"#)]
#[case::empty_id(r#"{"id": "", "function": {"name": "f", "arguments": "{}"}}"#)]
#[case::numeric_id(r#"{"id": 5, "function": {"name": "f", "arguments": "{}"}}"#)]
#[case::missing_function(r#"{"id": "call-1"}"#)]
#[case::missing_name(r#"{"id": "call-1", "function": {"arguments": "{}"}}"#)]
#[case::empty_name(r#"{"id": "call-1", "function": {"name": "", "arguments": "{}"}}"#)]
#[case::missing_arguments(r#"{"id": "call-1", "function": {"name": "f"}}"#)]
#[case::null_arguments(r#"{"id": "call-1", "function": {"name": "f", "arguments": null}}"#)]
#[case::not_an_object("\"call-1\"")]
fn a_tool_call_without_its_parts_is_dropped(#[case] tool_call: &str) {
    let body = format!(r#"{{"texts": ["x"], "tool_calls": [{tool_call}]}}"#);
    let payload = sent(&body, "{}", GuardrailInputType::Response, false);
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "response", "messages": null, "images": null, "tools": null,
                "response": {"role": "assistant", "content": "x", "tool_calls": []},
                "metadata": null}"#
        )
    );
}

#[rstest]
#[case::missing_type(r#"{"id": "call-1", "function": {"name": "f", "arguments": "{}"}}"#)]
#[case::empty_type(r#"{"id": "call-1", "type": "", "function": {"name": "f", "arguments": "{}"}}"#)]
#[case::numeric_type(
    r#"{"id": "call-1", "type": 5, "function": {"name": "f", "arguments": "{}"}}"#
)]
fn a_tool_call_without_a_usable_type_defaults_to_function(#[case] tool_call: &str) {
    let body = format!(r#"{{"texts": ["x"], "tool_calls": [{tool_call}]}}"#);
    let payload = sent(&body, "{}", GuardrailInputType::Response, false);
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "response", "messages": null, "images": null, "tools": null,
                "response": {"role": "assistant", "content": "x",
                             "tool_calls": [{"id": "call-1", "type": "function",
                                             "function": {"name": "f", "arguments": "{}"}}]},
                "metadata": null}"#
        )
    );
}

#[test]
fn object_arguments_are_written_as_json_text() {
    let payload = sent(
        r#"{"texts": ["x"], "tool_calls": [{"id": "call-1",
            "function": {"name": "f", "arguments": {"a": 1}}}]}"#,
        "{}",
        GuardrailInputType::Response,
        false,
    );
    assert_eq!(
        support::encoded(&payload),
        support::json_value(
            r#"{"correlation_id": null, "model_name": null, "model_provider_name": null,
                "guardrail_scope": "response", "messages": null, "images": null, "tools": null,
                "response": {"role": "assistant", "content": "x",
                             "tool_calls": [{"id": "call-1", "type": "function",
                                             "function": {"name": "f", "arguments": "{\"a\":1}"}}]},
                "metadata": null}"#
        )
    );
}

#[test]
fn the_logging_object_call_type_wins_over_the_request_call_type() {
    let logging = SingulrLoggingObject {
        call_type: Some("call_mcp_tool"),
    };
    assert!(is_mcp_call(
        &support::data(r#"{"call_type": "completion", "mcp_tool_name": "t"}"#),
        Some(&logging)
    ));
}

#[test]
fn a_logging_object_without_a_call_type_ignores_the_request_call_type() {
    let logging = SingulrLoggingObject { call_type: None };
    assert!(!is_mcp_call(
        &support::data(r#"{"call_type": "call_mcp_tool"}"#),
        Some(&logging)
    ));
}

#[test]
fn the_request_call_type_decides_without_a_logging_object() {
    let request_data = support::data(r#"{"call_type": "call_mcp_tool"}"#);
    assert!(is_mcp_call(&request_data, None));
    let other = support::data(r#"{"call_type": "completion", "mcp_tool_name": "t"}"#);
    assert!(!is_mcp_call(&other, None));
}

#[rstest]
#[case::empty_call_type(r#"{"call_type": ""}"#, false)]
#[case::null_call_type(r#"{"call_type": null, "mcp_tool_name": "t"}"#, true)]
#[case::numeric_call_type(r#"{"call_type": 5, "mcp_tool_name": "t"}"#, false)]
#[case::mcp_tool_name(r#"{"mcp_tool_name": null}"#, true)]
#[case::mcp_model_prefix(r#"{"model": "MCP: files"}"#, true)]
#[case::other_model(r#"{"model": "gpt-4o"}"#, false)]
fn mcp_detection_reads_the_request_fields(#[case] request_data: &str, #[case] expected: bool) {
    assert_eq!(is_mcp_call(&support::data(request_data), None), expected);
}
