#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailToolParam};
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::hiddenlayer::evaluation::tests::fixtures::{
    canonical, hl_headers, message, object, proxy_data, text, text_inputs,
};
use crate::hiddenlayer::evaluation::tests::{fixtures, server, tokens};
use crate::hiddenlayer::hiddenlayer_authentication::error::HiddenlayerError;

use rstest::rstest;

use super::writeback::last_choice_content;
use super::*;

fn json(value: &str) -> JsonValue {
    serde_json::from_str(value).expect("JSON")
}

fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).expect("JSON")
}

const ROUNDTRIP_ID: &str = "11111111-1111-1111-1111-111111111111";

fn fixed_id() -> Uuid {
    Uuid::parse_str(ROUNDTRIP_ID).expect("a uuid")
}

fn build(
    args: &EvaluationArgs,
    environment: &BTreeMap<String, String>,
    client: Client,
    tokens: Arc<tokens::ScriptedTokens>,
) -> HiddenlayerV2Guardrail {
    HiddenlayerV2Guardrail::new(args, environment, client, tokens, fixed_id).expect("a guardrail")
}

fn guardrail(base: &str) -> HiddenlayerV2Guardrail {
    build(
        &fixtures::args(base),
        &fixtures::environment(),
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
    )
}

#[test]
fn settings_have_no_supported_hooks() {
    let guardrail = guardrail("http://127.0.0.1:1");
    let settings = &guardrail.base.settings;
    assert_eq!(settings.supported_event_hooks, None);
    assert_eq!(settings.guardrail_name.as_deref(), Some("hiddenlayer-test"));
    assert!(settings.switches.uses_apply_guardrail_interface);
}

#[test]
fn invalid_timeout_is_rejected() {
    let args = EvaluationArgs {
        timeout: Some(-1.0_f64),
        ..fixtures::args("http://127.0.0.1:1")
    };
    let result = HiddenlayerV2Guardrail::new(
        &args,
        &fixtures::environment(),
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
        fixed_id,
    );
    assert!(matches!(
        result,
        Err(EvaluationError::Auth(
            HiddenlayerError::InvalidTimeout { .. }
        ))
    ));
}

#[test]
fn request_payload_carries_messages_model_and_tools() {
    let inputs = GuardrailApiInputs {
        texts: None,
        images: None,
        tools: Some(vec![GuardrailToolParam {
            tool_type: "function".to_owned(),
            extra: BTreeMap::from([(
                "function".to_owned(),
                object(vec![("name", text("lookup"))]),
            )]),
        }]),
        tool_calls: None,
        structured_messages: Some(vec![message("user", text("hi"))]),
        model: Some("gpt-4o".to_owned()),
        stream_holdback_chars: None,
    };
    assert_eq!(
        request_payload(&inputs).expect("a payload"),
        json(
            r#"{"messages": [{"role": "user", "content": "hi"}], "model": "gpt-4o",
                "tools": [{"type": "function", "function": {"name": "lookup"}}]}"#
        )
    );
}

#[test]
fn request_payload_sends_nulls_for_absent_parts() {
    assert_eq!(
        request_payload(&GuardrailApiInputs::default()).expect("a payload"),
        json(r#"{"messages": null, "model": null, "tools": null}"#)
    );
}

#[test]
fn response_payload_wraps_the_first_text_in_a_choice() {
    let inputs = text_inputs(&["answer", "second"]);
    assert_eq!(
        response_payload(&inputs).expect("a payload"),
        json(
            r#"{"choices": [{"index": 0, "message": {"role": "assistant", "content": "answer"},
                "finish_reason": "stop"}]}"#
        )
    );
}

#[test]
fn response_payload_uses_tool_calls_without_texts() {
    let inputs = GuardrailApiInputs {
        texts: Some(Vec::new()),
        images: None,
        tools: None,
        tool_calls: Some(vec![object(vec![
            ("id", text("call-1")),
            ("type", text("function")),
        ])]),
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    assert_eq!(
        response_payload(&inputs).expect("a payload"),
        json(r#"[{"id": "call-1", "type": "function"}]"#)
    );
}

#[test]
fn response_payload_without_choices_or_tool_calls_is_empty() {
    assert_eq!(
        response_payload(&GuardrailApiInputs::default()).expect("a payload"),
        json("{}")
    );
    assert_eq!(
        response_payload(&GuardrailApiInputs {
            texts: None,
            images: None,
            tools: None,
            tool_calls: Some(Vec::new()),
            structured_messages: None,
            model: None,
            stream_holdback_chars: None,
        })
        .expect("a payload"),
        json("{}")
    );
}

#[test]
fn roundtrip_headers_default_the_requester_and_inject_an_id() {
    let headers = BTreeMap::from([
        ("hl-project-id".to_owned(), "project".to_owned()),
        ("x-other".to_owned(), "dropped".to_owned()),
    ]);
    let mut data = proxy_data(&[("hl-project-id", "project")]);
    let prepared = roundtrip_headers(&headers, &mut data, fixed_id);
    assert_eq!(
        prepared,
        BTreeMap::from([
            ("hl-project-id".to_owned(), "project".to_owned()),
            ("hl-requester-id".to_owned(), "LiteLLM".to_owned()),
            ("hl-roundtrip-id".to_owned(), ROUNDTRIP_ID.to_owned()),
        ])
    );
    assert_eq!(
        metadata::proxy_request_headers(&data),
        BTreeMap::from([
            ("hl-project-id".to_owned(), "project".to_owned()),
            ("hl-roundtrip-id".to_owned(), ROUNDTRIP_ID.to_owned()),
        ])
    );
}

#[test]
fn roundtrip_headers_keep_present_values() {
    let headers = BTreeMap::from([
        ("hl-requester-id".to_owned(), String::new()),
        ("hl-roundtrip-id".to_owned(), "kept".to_owned()),
    ]);
    let mut data = proxy_data(&[("hl-roundtrip-id", "kept")]);
    assert_eq!(
        roundtrip_headers(&headers, &mut data, fixed_id),
        BTreeMap::from([
            ("hl-requester-id".to_owned(), String::new()),
            ("hl-roundtrip-id".to_owned(), "kept".to_owned()),
        ])
    );
    assert_eq!(
        metadata::proxy_request_headers(&data),
        BTreeMap::from([("hl-roundtrip-id".to_owned(), "kept".to_owned())])
    );
    let mut mixed = proxy_data(&[("HL-Roundtrip-Id", "Upper")]);
    let lowered = metadata::proxy_request_headers(&mixed);
    let prepared = roundtrip_headers(&lowered, &mut mixed, fixed_id);
    assert_eq!(
        prepared.get("hl-roundtrip-id").map(String::as_str),
        Some("Upper")
    );
}

#[test]
fn roundtrip_headers_need_a_proxy_request_to_inject_an_id() {
    let mut data = BTreeMap::new();
    assert_eq!(
        roundtrip_headers(&BTreeMap::new(), &mut data, fixed_id),
        BTreeMap::from([("hl-requester-id".to_owned(), "LiteLLM".to_owned())])
    );
    assert!(data.is_empty());
}

#[test]
fn detection_headers_let_the_hl_headers_win() {
    let hl_headers = BTreeMap::from([
        ("hl-project-id".to_owned(), "project".to_owned()),
        (
            "hl-runtime-edge-provider-version".to_owned(),
            "9".to_owned(),
        ),
    ]);
    let headers = detection_headers(VERSION, None, Some(&hl_headers)).expect("headers");
    assert_eq!(
        headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        headers
            .get("hl-runtime-edge-provider")
            .and_then(|value| value.to_str().ok()),
        Some("litellm")
    );
    assert_eq!(
        headers
            .get("hl-runtime-edge-provider-version")
            .and_then(|value| value.to_str().ok()),
        Some("9")
    );
    assert_eq!(
        headers
            .get("hl-project-id")
            .and_then(|value| value.to_str().ok()),
        Some("project")
    );
}

#[test]
fn write_request_rebuilds_the_texts_and_messages() {
    let mut inputs = text_inputs(&["old"]);
    let output = value(
        r#"{"messages": [
            {"role": "system", "content": "a"},
            {"content": [{"type": "text", "text": "b"}, {"type": "text", "text": "c"}]},
            {"role": "user", "content": ""},
            {"role": "user", "content": [{"type": "image_url"}]},
            {"role": "user", "content": [{"type": "text"}]}
        ]}"#,
    );
    write_request(&mut inputs, &output).expect("written");
    assert_eq!(inputs.texts, Some(vec!["a".to_owned(), "b c".to_owned()]));
    let messages = inputs.structured_messages.expect("messages");
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[2].role, "user");
    assert_eq!(messages[3].role, "user");
    assert_eq!(messages.len(), 5);
    assert_eq!(
        messages[1].extra.get("content"),
        Some(&value(
            r#"[{"type": "text", "text": "b"}, {"type": "text", "text": "c"}]"#
        ))
    );
}

#[rstest]
#[case::other_keys(r#"{"other": 1}"#)]
#[case::null_messages(r#"{"messages": null}"#)]
fn write_request_without_messages_clears_both_lists(#[case] body: &str) {
    let mut inputs = text_inputs(&["old"]);
    write_request(&mut inputs, &value(body)).expect("written");
    assert_eq!(inputs.texts, Some(Vec::new()));
    assert_eq!(inputs.structured_messages, Some(Vec::new()));
}

#[test]
fn write_request_rejects_a_shape_it_cannot_read() {
    assert!(matches!(
        write_request(&mut text_inputs(&["old"]), &text("not an object")),
        Err(EvaluationError::Shape { .. })
    ));
    let body = value(r#"{"messages": ["plain"]}"#);
    assert!(matches!(
        write_request(&mut text_inputs(&["old"]), &body),
        Err(EvaluationError::Shape { .. })
    ));
}

#[test]
fn write_response_takes_the_last_choice_content() {
    let mut inputs = text_inputs(&["old"]);
    let output = value(
        r#"{"choices": [{"message": {"content": "first"}}, {"message": {"content": "redacted"}}]}"#,
    );
    write_response(&mut inputs, &output).expect("written");
    assert_eq!(inputs.texts, Some(vec!["redacted".to_owned()]));
}

#[test]
fn last_choice_content_reads_only_a_text_content() {
    assert_eq!(
        last_choice_content(&value("{}")).expect("content"),
        String::new()
    );
    assert_eq!(
        last_choice_content(&value(r#"{"choices": [{"message": {}}]}"#)).expect("content"),
        String::new()
    );
    assert_eq!(
        last_choice_content(&value(r#"{"choices": null}"#)).expect("content"),
        String::new()
    );
    assert_eq!(
        last_choice_content(&value(r#"{"choices": [{"message": {"content": null}}]}"#))
            .expect("content"),
        String::new()
    );
    assert_eq!(
        last_choice_content(&value(r#"{"choices": [{"message": {"content": "c"}}]}"#))
            .expect("content"),
        "c"
    );
}

#[rstest]
#[case::plain_text(r#""plain""#)]
#[case::empty_choices(r#"{"choices": []}"#)]
#[case::plain_choice(r#"{"choices": ["plain"]}"#)]
#[case::numeric_content(r#"{"choices": [{"message": {"content": 3}}]}"#)]
#[case::plain_message(r#"{"choices": [{"message": "plain"}]}"#)]
fn last_choice_content_without_a_choice_is_a_shape_error(#[case] body: &str) {
    let output = value(body);
    assert!(
        matches!(
            last_choice_content(&output),
            Err(EvaluationError::Shape { .. })
        ),
        "{output:?}"
    );
}

#[test]
fn write_response_replaces_only_the_tool_calls() {
    let mut inputs = GuardrailApiInputs {
        texts: Some(Vec::new()),
        images: None,
        tools: None,
        tool_calls: Some(vec![text("old")]),
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    let output = value(r#"[{"id": "new"}]"#);
    write_response(&mut inputs, &output).expect("written");
    assert_eq!(inputs.tool_calls, Some(vec![value(r#"{"id": "new"}"#)]));
    assert_eq!(inputs.texts, Some(Vec::new()));
}

#[test]
fn a_response_replacing_tool_calls_must_be_a_list() {
    let mut inputs = GuardrailApiInputs {
        texts: Some(Vec::new()),
        images: None,
        tools: None,
        tool_calls: Some(vec![text("old")]),
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    assert!(matches!(
        write_response(&mut inputs, &value(r#"{"id": "new"}"#)),
        Err(EvaluationError::Shape { .. })
    ));
}

#[test]
fn write_response_fails_on_an_empty_choice_list() {
    assert!(matches!(
        write_response(&mut text_inputs(&["old"]), &value(r#"{"choices": []}"#)),
        Err(EvaluationError::Shape { .. })
    ));
}

#[test]
fn write_response_without_scanned_output_keeps_the_inputs() {
    let mut inputs = GuardrailApiInputs::default();
    write_response(&mut inputs, &value("{}")).expect("written");
    assert_eq!(inputs, GuardrailApiInputs::default());
    let mut empty_calls = GuardrailApiInputs {
        texts: None,
        images: None,
        tools: None,
        tool_calls: Some(Vec::new()),
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    write_response(&mut empty_calls, &value("{}")).expect("written");
    assert_eq!(empty_calls.tool_calls, Some(Vec::new()));
}

fn request_inputs() -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: None,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: Some(vec![message("user", text("secret"))]),
        model: Some("gpt-4o".to_owned()),
        stream_holdback_chars: None,
    }
}

#[tokio::test]
async fn sends_the_v2_request_evaluation() {
    let server = server::start(vec![server::reply(
        r#"{"messages": [{"role": "user", "content": "redacted"}]}"#,
    )]);
    let guardrail = guardrail(&server.base);
    let mut data = proxy_data(&[("hl-project-id", "project")]);
    let evaluated = guardrail
        .evaluate(
            request_inputs(),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("evaluated");
    assert_eq!(
        evaluated.structured_messages,
        Some(vec![message("user", text("redacted"))])
    );
    let seen = server.requests();
    assert_eq!(seen[0].path, "/detection/v2/request-evaluations");
    assert_eq!(
        hl_headers(&seen[0]),
        BTreeMap::from([
            ("hl-project-id".to_owned(), "project".to_owned()),
            ("hl-requester-id".to_owned(), "LiteLLM".to_owned()),
            ("hl-roundtrip-id".to_owned(), ROUNDTRIP_ID.to_owned()),
            ("hl-runtime-edge-provider".to_owned(), "litellm".to_owned()),
            (
                "hl-runtime-edge-provider-version".to_owned(),
                "2".to_owned()
            ),
        ])
    );
    assert_eq!(
        canonical(&seen[0].body),
        r#"{"messages":[{"role":"user","content":"secret"}],"model":"gpt-4o","tools":null}"#
    );
}

#[tokio::test]
async fn sends_the_v2_response_evaluation() {
    let server = server::start(vec![server::reply(
        r#"{"choices": [{"message": {"content": "redacted"}}]}"#,
    )]);
    let guardrail = guardrail(&server.base);
    let mut data = BTreeMap::new();
    let evaluated = guardrail
        .evaluate(
            text_inputs(&["answer"]),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("evaluated");
    assert_eq!(evaluated.texts, Some(vec!["redacted".to_owned()]));
    let seen = server.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/detection/v2/response-evaluations");
    assert_eq!(
        hl_headers(&seen[0]),
        BTreeMap::from([
            ("hl-requester-id".to_owned(), "LiteLLM".to_owned()),
            ("hl-runtime-edge-provider".to_owned(), "litellm".to_owned()),
            (
                "hl-runtime-edge-provider-version".to_owned(),
                "2".to_owned()
            ),
        ])
    );
    assert_eq!(
        canonical(&seen[0].body),
        r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}]}"#
    );
}

#[tokio::test]
async fn blocks_on_an_upper_case_action_header() {
    let server = server::start(vec![server::reply_with(
        200,
        &[("hl-runtime-action", "BLOCK")],
        "{}",
    )]);
    let guardrail = guardrail(&server.base);
    let result = guardrail
        .evaluate(
            text_inputs(&["answer"]),
            &mut BTreeMap::new(),
            GuardrailInputType::Response,
            None,
        )
        .await;
    assert!(matches!(
        result,
        Err(EvaluationError::Blocked { status: 400, .. })
    ));
}

#[tokio::test]
async fn blocks_on_a_mixed_case_action_header() {
    let server = server::start(vec![server::reply_with(
        200,
        &[("hl-runtime-action", "Block")],
        "{}",
    )]);
    let guardrail = guardrail(&server.base);
    let result = guardrail
        .evaluate(
            text_inputs(&["answer"]),
            &mut BTreeMap::new(),
            GuardrailInputType::Response,
            None,
        )
        .await;
    let error = result.expect_err("a block");
    let EvaluationError::Blocked { status, detail } = error else {
        panic!("a block");
    };
    assert_eq!(status, 400);
    assert_eq!(detail.len(), 2);
    assert_eq!(detail.get("error"), Some(&text(VIOLATED_POLICY)));
    assert_eq!(
        detail.get("hiddenlayer_guardrail_response"),
        Some(&text(BLOCK_MESSAGE))
    );
}

#[tokio::test]
async fn another_action_header_does_not_block() {
    let server = server::start(vec![server::reply_with(
        200,
        &[("hl-runtime-action", "allow")],
        r#"{"choices": [{"message": {"content": "kept"}}]}"#,
    )]);
    let guardrail = guardrail(&server.base);
    let evaluated = guardrail
        .evaluate(
            text_inputs(&["answer"]),
            &mut BTreeMap::new(),
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("evaluated");
    assert_eq!(evaluated.texts, Some(vec!["kept".to_owned()]));
}

#[tokio::test]
async fn a_non_json_answer_is_a_v2_decode_error() {
    let server = server::start(vec![server::reply("not json")]);
    let guardrail = guardrail(&server.base);
    let result = guardrail
        .evaluate(
            text_inputs(&["answer"]),
            &mut BTreeMap::new(),
            GuardrailInputType::Response,
            None,
        )
        .await;
    assert!(matches!(result, Err(EvaluationError::Decode { .. })));
}

#[tokio::test]
async fn refreshes_the_token_on_401_and_retries() {
    let server = server::start(vec![
        server::reply_with(401, &[], "{}"),
        server::reply(r#"{"choices": [{"message": {"content": "fresh"}}]}"#),
    ]);
    let tokens = Arc::new(tokens::ScriptedTokens::new(&["stale", "fresh"]));
    let args = fixtures::args(&fixtures::saas_base(server.port));
    let guardrail = build(
        &args,
        &fixtures::environment(),
        fixtures::saas_client(server.port),
        Arc::clone(&tokens),
    );
    let evaluated = guardrail
        .evaluate(
            text_inputs(&["answer"]),
            &mut BTreeMap::new(),
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("evaluated");
    assert_eq!(evaluated.texts, Some(vec!["fresh".to_owned()]));
    let seen = server.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer stale")
    );
    assert_eq!(
        seen[1].headers.get("authorization").map(String::as_str),
        Some("Bearer fresh")
    );
    assert_eq!(tokens.fetches().len(), 2);
}
