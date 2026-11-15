#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use reqwest::header::HeaderValue;
use uuid::Uuid;

use crate::hiddenlayer::evaluation::tests::fixtures::{
    canonical, headers_value, logging_data, message, object, proxy_data, text, text_inputs,
};
use crate::hiddenlayer::evaluation::tests::{fixtures, server, tokens};
use crate::hiddenlayer::hiddenlayer_authentication::error::HiddenlayerError;
use crate::hiddenlayer::hiddenlayer_v2::roundtrip_headers;

use super::response::*;
use super::*;

fn interaction(json: &str) -> InteractionBody {
    serde_json::from_str(json).expect("an interaction body")
}

fn build(
    args: &EvaluationArgs,
    environment: &BTreeMap<String, String>,
    client: Client,
    tokens: Arc<tokens::ScriptedTokens>,
) -> HiddenlayerV1Guardrail {
    HiddenlayerV1Guardrail::new(args, environment, client, tokens).expect("a guardrail")
}

fn guardrail(base: &str) -> HiddenlayerV1Guardrail {
    build(
        &fixtures::args(base),
        &fixtures::environment(),
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
    )
}

#[test]
fn settings_carry_name_hook_and_default_on() {
    let args = EvaluationArgs {
        event_hook: LitellmParamsMode::One("post_call".to_owned()),
        default_on: true,
        ..fixtures::args("http://127.0.0.1:1")
    };
    let guardrail = build(
        &args,
        &fixtures::environment(),
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
    );
    let settings = &guardrail.base.settings;
    assert_eq!(settings.guardrail_name.as_deref(), Some("hiddenlayer-test"));
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall
        ])
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
    assert!(settings.default_on);
    assert!(settings.switches.uses_apply_guardrail_interface);
}

#[test]
fn unsupported_event_hook_is_rejected() {
    let args = EvaluationArgs {
        event_hook: LitellmParamsMode::One("during_call".to_owned()),
        ..fixtures::args("http://127.0.0.1:1")
    };
    let result = HiddenlayerV1Guardrail::new(
        &args,
        &fixtures::environment(),
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
    );
    assert!(matches!(result, Err(EvaluationError::Base(_))));
}

#[test]
fn invalid_timeout_is_rejected() {
    let args = EvaluationArgs {
        timeout: Some(-1.0),
        ..fixtures::args("http://127.0.0.1:1")
    };
    let result = HiddenlayerV1Guardrail::new(
        &args,
        &fixtures::environment(),
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
    );
    assert!(matches!(
        result,
        Err(EvaluationError::Auth(
            HiddenlayerError::InvalidTimeout { .. }
        ))
    ));
}

#[test]
fn evaluation_timeout_defaults_to_thirty_seconds() {
    assert_eq!(
        evaluation_timeout(None).expect("a default"),
        Duration::from_secs(30)
    );
    assert_eq!(
        evaluation_timeout(Some(2.5)).expect("a timeout"),
        Duration::from_millis(2_500)
    );
    assert!(evaluation_timeout(Some(-1.0)).is_err());
}

#[test]
fn scan_payload_scans_the_last_structured_message() {
    let inputs = GuardrailApiInputs {
        structured_messages: Some(vec![
            message("user", text("first")),
            message("assistant", text("second")),
        ]),
        texts: Some(vec!["ignored".to_owned()]),
        ..GuardrailApiInputs::default()
    };
    let payload = scan_payload(&inputs).expect("a payload");
    assert_eq!(
        payload,
        ScanMessage {
            role: "assistant".to_owned(),
            content: "second".to_owned(),
        }
    );
}

#[test]
fn scan_payload_flattens_text_parts() {
    let content = GuardrailValue::Array(vec![
        object(vec![("type", text("text")), ("text", text("a"))]),
        object(vec![("type", text("image_url"))]),
        object(vec![("type", text("text")), ("text", text("b"))]),
    ]);
    let inputs = GuardrailApiInputs {
        structured_messages: Some(vec![GuardrailMessage {
            role: "user".to_owned(),
            extra: BTreeMap::from([("content".to_owned(), content)]),
        }]),
        ..GuardrailApiInputs::default()
    };
    let payload = scan_payload(&inputs).expect("a payload");
    assert_eq!(
        payload,
        ScanMessage {
            role: "user".to_owned(),
            content: "a\n\nb".to_owned(),
        }
    );
}

#[test]
fn scan_payload_message_without_content_scans_empty_text() {
    let inputs = GuardrailApiInputs {
        structured_messages: Some(vec![GuardrailMessage {
            role: "user".to_owned(),
            extra: BTreeMap::new(),
        }]),
        ..GuardrailApiInputs::default()
    };
    let payload = scan_payload(&inputs).expect("a payload");
    assert_eq!(
        payload,
        ScanMessage {
            role: "user".to_owned(),
            content: String::new(),
        }
    );
}

#[test]
fn scan_payload_falls_back_to_the_last_text() {
    let payload = scan_payload(&text_inputs(&["first", "second"])).expect("a payload");
    assert_eq!(
        payload,
        ScanMessage {
            role: "user".to_owned(),
            content: "second".to_owned(),
        }
    );
    let empty_messages = GuardrailApiInputs {
        structured_messages: Some(Vec::new()),
        ..text_inputs(&["only"])
    };
    assert_eq!(
        scan_payload(&empty_messages).expect("a payload"),
        ScanMessage {
            role: "user".to_owned(),
            content: "only".to_owned(),
        }
    );
}

#[test]
fn scan_payload_without_inputs_is_none() {
    assert!(scan_payload(&GuardrailApiInputs::default()).is_none());
    assert!(
        scan_payload(&GuardrailApiInputs {
            texts: Some(Vec::new()),
            ..GuardrailApiInputs::default()
        })
        .is_none()
    );
}

#[tokio::test]
async fn empty_metadata_headers_fall_back_to_the_defaults() {
    let server = server::start(vec![server::reply("{}")]);
    let guardrail = guardrail(&server.base);
    guardrail
        .evaluate(
            text_inputs(&["hello"]),
            &proxy_data(&[("hl-requester-id", ""), ("hl-project-id", "")]),
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("evaluated");
    let seen = server.requests();
    assert!(seen[0].headers.get("hl-project-id").is_none());
    assert_eq!(
        canonical(&seen[0].body),
        r#"{"metadata":{"model":"unknown","requester_id":"LiteLLM"},"input":{"messages":[{"role":"user","content":"hello"}]}}"#
    );
}

#[test]
fn evaluation_model_reads_the_logging_object() {
    assert_eq!(
        metadata::evaluation_model(&logging_data("gpt-4o")),
        "gpt-4o"
    );
    assert_eq!(
        metadata::evaluation_model(&BTreeMap::from([(
            "litellm_logging_obj".to_owned(),
            object(vec![("model", text(""))])
        )])),
        "unknown"
    );
    assert_eq!(
        metadata::evaluation_model(&BTreeMap::from([(
            "litellm_logging_obj".to_owned(),
            object(vec![("model", GuardrailValue::Null)])
        )])),
        "unknown"
    );
    assert_eq!(
        metadata::evaluation_model(&BTreeMap::from([(
            "litellm_logging_obj".to_owned(),
            text("not an object")
        )])),
        "unknown"
    );
    assert_eq!(metadata::evaluation_model(&BTreeMap::new()), "unknown");
}

#[test]
fn proxy_headers_lowercase_names_and_drop_non_text_values() {
    let data = BTreeMap::from([(
        "proxy_server_request".to_owned(),
        object(vec![(
            "headers",
            object(vec![
                ("X-HL-Project-Id", text("project")),
                ("count", GuardrailValue::UnsignedInteger(3)),
            ]),
        )]),
    )]);
    assert_eq!(
        metadata::proxy_request_headers(&data),
        BTreeMap::from([("x-hl-project-id".to_owned(), "project".to_owned())])
    );
    assert!(metadata::proxy_request_headers(&BTreeMap::new()).is_empty());
}

#[test]
fn logged_headers_come_from_the_model_call_params() {
    let target = GuardrailLogTarget {
        litellm_params: None,
        model_call_params: Some(BTreeMap::from([(
            "metadata".to_owned(),
            object(vec![(
                "headers",
                headers_value(&[("HL-Requester-Id", "log")]),
            )]),
        )])),
    };
    assert_eq!(
        metadata::logged_request_headers(Some(&target)),
        BTreeMap::from([("hl-requester-id".to_owned(), "log".to_owned())])
    );
    assert!(metadata::logged_request_headers(Some(&GuardrailLogTarget::default())).is_empty());
    assert!(metadata::logged_request_headers(None).is_empty());
}

#[test]
fn request_headers_prefer_the_proxy_request() {
    let target = GuardrailLogTarget {
        litellm_params: None,
        model_call_params: Some(BTreeMap::from([(
            "metadata".to_owned(),
            object(vec![(
                "headers",
                headers_value(&[("hl-requester-id", "log")]),
            )]),
        )])),
    };
    assert_eq!(
        metadata::request_headers(&proxy_data(&[("hl-requester-id", "proxy")]), Some(&target)),
        BTreeMap::from([("hl-requester-id".to_owned(), "proxy".to_owned())])
    );
    assert_eq!(
        metadata::request_headers(&BTreeMap::new(), Some(&target)),
        BTreeMap::from([("hl-requester-id".to_owned(), "log".to_owned())])
    );
}

#[test]
fn hl_headers_keep_only_hl_prefixed_names() {
    let headers = BTreeMap::from([
        ("hl-project-id".to_owned(), "project".to_owned()),
        ("x-project-id".to_owned(), "other".to_owned()),
        ("hl-roundtrip-id".to_owned(), "id".to_owned()),
        ("hl-requester-id".to_owned(), "app".to_owned()),
    ]);
    assert_eq!(
        roundtrip_headers(&headers, &mut BTreeMap::new(), Uuid::nil),
        BTreeMap::from([
            ("hl-project-id".to_owned(), "project".to_owned()),
            ("hl-roundtrip-id".to_owned(), "id".to_owned()),
            ("hl-requester-id".to_owned(), "app".to_owned()),
        ])
    );
}

#[test]
fn roundtrip_headers_write_the_proxy_headers() {
    let mut data = proxy_data(&[("hl-project-id", "project")]);
    let id = Uuid::nil().to_string();
    let headers = roundtrip_headers(&BTreeMap::new(), &mut data, Uuid::nil);
    assert_eq!(headers.get("hl-roundtrip-id"), Some(&id));
    assert_eq!(
        metadata::proxy_request_headers(&data),
        BTreeMap::from([
            ("hl-project-id".to_owned(), "project".to_owned()),
            ("hl-roundtrip-id".to_owned(), id),
        ])
    );
    let mut without_headers = BTreeMap::from([(
        "proxy_server_request".to_owned(),
        object(vec![("body", text("{}"))]),
    )]);
    let kept = roundtrip_headers(&BTreeMap::new(), &mut without_headers, Uuid::nil);
    assert!(!kept.contains_key("hl-roundtrip-id"));
}

#[test]
fn header_value_is_case_insensitive_and_defaulted() {
    let mut headers = HeaderMap::new();
    headers.insert("hl-runtime-action", HeaderValue::from_static("Block"));
    assert_eq!(
        metadata::header_value(&headers, "hl-runtime-action", ""),
        "Block"
    );
    assert_eq!(
        metadata::header_value(&headers, "hl-missing", "absent"),
        "absent"
    );
    let mut opaque = HeaderMap::new();
    opaque.insert(
        "hl-runtime-action",
        HeaderValue::from_bytes(b"\xff").expect("an opaque value"),
    );
    assert_eq!(
        metadata::header_value(&opaque, "hl-runtime-action", "absent"),
        "absent"
    );
}

#[test]
fn apply_result_allows_an_unknown_action() {
    let inputs = text_inputs(&["hello"]);
    assert_eq!(
        apply_result(
            inputs.clone(),
            &interaction(r#"{"evaluation": {"action": "Allow"}}"#),
            GuardrailInputType::Request
        )
        .expect("allowed"),
        inputs
    );
    assert_eq!(
        apply_result(
            inputs.clone(),
            &interaction(r#"{"evaluation": {}}"#),
            GuardrailInputType::Request
        )
        .expect("allowed"),
        inputs
    );
    assert_eq!(
        apply_result(
            inputs.clone(),
            &interaction("{}"),
            GuardrailInputType::Request
        )
        .expect("allowed"),
        inputs
    );
}

#[test]
fn apply_result_block_builds_the_violation_detail() {
    let body = interaction(
        r#"{"evaluation": {"action": "Block", "threat_level": "high"},
            "analysis": [
                {"name": "prompt_injection", "detected": true},
                {"name": "pii", "detected": false},
                {"detected": true}
            ]}"#,
    );
    let error = apply_result(text_inputs(&["hello"]), &body, GuardrailInputType::Request)
        .expect_err("blocked");
    let EvaluationError::Blocked { status, detail } = error else {
        panic!("a block");
    };
    assert_eq!(status, 400);
    assert_eq!(detail.get("error"), Some(&text(VIOLATED_POLICY)));
    assert_eq!(
        detail.get("hiddenlayer_guardrail_response"),
        Some(&text(BLOCK_MESSAGE))
    );
    assert_eq!(
        detail.get("block_reasons"),
        Some(&GuardrailValue::Array(vec![
            text("prompt_injection"),
            text("unknown"),
        ]))
    );
    assert_eq!(detail.get("threat_level"), Some(&text("high")));
}

#[test]
fn block_without_analysis_reports_an_empty_reason_list() {
    let body = interaction(r#"{"evaluation": {"action": "Block"}}"#);
    let error = apply_result(text_inputs(&["hello"]), &body, GuardrailInputType::Response)
        .expect_err("blocked");
    let EvaluationError::Blocked { status, detail } = error else {
        panic!("a block");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("block_reasons"),
        Some(&GuardrailValue::Array(Vec::new()))
    );
    assert_eq!(detail.get("threat_level"), Some(&GuardrailValue::Null));
}

#[test]
fn apply_result_redact_rewrites_the_request_messages() {
    let body = interaction(
        r#"{"evaluation": {"action": "Redact"},
            "modified_data": {
                "input": {"messages": [{"content": "first"}, {"role": "user", "content": "second"}]},
                "output": {"messages": [{"role": "assistant", "content": "out"}]}
            }}"#,
    );
    let evaluated = apply_result(
        text_inputs(&["original"]),
        &body,
        GuardrailInputType::Request,
    )
    .expect("redacted");
    assert_eq!(evaluated.texts, Some(vec!["second".to_owned()]));
    let messages = evaluated.structured_messages.expect("messages");
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].extra.get("content"), Some(&text("first")));
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].extra.get("content"), Some(&text("second")));
}

#[test]
fn apply_result_redact_rewrites_only_the_response_texts() {
    let body = interaction(
        r#"{"evaluation": {"action": "Redact"},
            "modified_data": {
                "input": {"messages": [{"role": "user", "content": "in"}]},
                "output": {"messages": [{"role": "assistant", "content": "out"}]}
            }}"#,
    );
    let evaluated = apply_result(
        text_inputs(&["original"]),
        &body,
        GuardrailInputType::Response,
    )
    .expect("redacted");
    assert_eq!(evaluated.texts, Some(vec!["out".to_owned()]));
    assert_eq!(evaluated.structured_messages, None);
}

#[test]
fn redact_without_modified_messages_keeps_the_inputs() {
    let inputs = text_inputs(&["hello"]);
    for body in [
        r#"{"evaluation": {"action": "Redact"}}"#,
        r#"{"evaluation": {"action": "Redact"}, "modified_data": {}}"#,
        r#"{"evaluation": {"action": "Redact"}, "modified_data": {"input": null}}"#,
        r#"{"evaluation": {"action": "Redact"}, "modified_data": {"output": {"messages": [{"content": "out"}]}}}"#,
        r#"{"evaluation": {"action": "Redact"}, "modified_data": {"input": {}}}"#,
    ] {
        assert_eq!(
            apply_result(
                inputs.clone(),
                &interaction(body),
                GuardrailInputType::Request
            )
            .expect("unchanged"),
            inputs
        );
    }
}

#[test]
fn redact_without_indexable_messages_is_a_shape_error() {
    for body in [
        r#"{"evaluation": {"action": "Redact"}, "modified_data": {"input": {"messages": []}}}"#,
        r#"{"evaluation": {"action": "Redact"}, "modified_data": {"input": {"other": 1}}}"#,
        r#"{"evaluation": {"action": "Redact"}, "modified_data": {"input": {"messages": null}}}"#,
    ] {
        assert!(
            matches!(
                apply_result(
                    text_inputs(&["hello"]),
                    &interaction(body),
                    GuardrailInputType::Request
                ),
                Err(EvaluationError::Shape { .. })
            ),
            "{body}"
        );
    }
    let empty_output = interaction(
        r#"{"evaluation": {"action": "Redact"}, "modified_data": {"output": {"messages": []}}}"#,
    );
    assert!(matches!(
        apply_result(
            text_inputs(&["hello"]),
            &empty_output,
            GuardrailInputType::Response
        ),
        Err(EvaluationError::Shape { .. })
    ));
}

#[test]
fn redacted_texts_read_content_parts() {
    let last = object(vec![(
        "content",
        GuardrailValue::Array(vec![
            object(vec![("type", text("text")), ("text", text("a"))]),
            object(vec![("type", text("image_url"))]),
            object(vec![("type", text("text")), ("text", text("b"))]),
        ]),
    )]);
    assert_eq!(
        redacted_texts(&last).expect("texts"),
        vec!["a".to_owned(), "b".to_owned()]
    );
    assert_eq!(
        redacted_texts(&object(vec![(
            "content",
            GuardrailValue::Array(vec![object(vec![("type", text("image_url"))])])
        )]))
        .expect("texts"),
        vec![String::new()]
    );
    assert_eq!(
        redacted_texts(&object(vec![("content", text("plain"))])).expect("texts"),
        vec!["plain".to_owned()]
    );
    assert_eq!(
        redacted_texts(&object(vec![(
            "content",
            GuardrailValue::Array(vec![
                object(vec![("type", text("text")), ("text", text("a"))]),
                object(vec![("type", text("text"))])
            ])
        )]))
        .expect("texts"),
        vec!["a".to_owned()]
    );
}

#[test]
fn redacted_texts_without_a_content_are_a_shape_error() {
    for message in [
        object(vec![("role", text("user"))]),
        object(vec![("content", GuardrailValue::Null)]),
        object(vec![("content", GuardrailValue::UnsignedInteger(3))]),
        text("not a message"),
    ] {
        assert!(matches!(
            redacted_texts(&message),
            Err(EvaluationError::Shape { .. })
        ));
    }
}

#[test]
fn redact_with_a_non_object_message_is_a_shape_error() {
    let body = interaction(
        r#"{"evaluation": {"action": "Redact"},
            "modified_data": {"input": {"messages": ["plain"]}}}"#,
    );
    assert!(matches!(
        apply_result(text_inputs(&["hello"]), &body, GuardrailInputType::Request),
        Err(EvaluationError::Shape { .. })
    ));
}

#[test]
fn as_guardrail_message_defaults_a_missing_role_to_user() {
    let converted = as_guardrail_message(&object(vec![
        ("role", text("assistant")),
        ("content", text("hello")),
        ("name", text("tool")),
    ]))
    .expect("a message");
    assert_eq!(converted.role, "assistant");
    assert_eq!(converted.extra.get("content"), Some(&text("hello")));
    assert_eq!(converted.extra.get("name"), Some(&text("tool")));
    let roleless =
        as_guardrail_message(&object(vec![("content", text("hello"))])).expect("a message");
    assert_eq!(roleless.role, "user");
    let numeric_role =
        as_guardrail_message(&object(vec![("role", GuardrailValue::UnsignedInteger(3))]))
            .expect("a message");
    assert_eq!(numeric_role.role, "user");
    assert!(as_guardrail_message(&text("plain")).is_err());
}

#[test]
fn detection_headers_name_the_provider_and_project() {
    let headers = detection_headers(VERSION, None, None).expect("headers");
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        headers
            .get("hl-runtime-edge-provider")
            .and_then(|v| v.to_str().ok()),
        Some("litellm")
    );
    assert_eq!(
        headers
            .get("hl-runtime-edge-provider-version")
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    assert!(headers.get("hl-project-id").is_none());
    let with_project = detection_headers(VERSION, Some("project"), None).expect("headers");
    assert_eq!(
        with_project
            .get("hl-project-id")
            .and_then(|v| v.to_str().ok()),
        Some("project")
    );
}

#[tokio::test]
async fn sends_the_v1_interaction_request() {
    let server = server::start(vec![server::reply("{}")]);
    let guardrail = guardrail(&server.base);
    let evaluated = guardrail
        .evaluate(
            text_inputs(&["hello"]),
            &BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("evaluated");
    assert_eq!(evaluated.texts, Some(vec!["hello".to_owned()]));
    let seen = server.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].path, "/detection/v1/interactions");
    assert!(seen[0].headers.get("authorization").is_none());
    assert_eq!(
        canonical(&seen[0].body),
        r#"{"metadata":{"model":"unknown","requester_id":"LiteLLM"},"input":{"messages":[{"role":"user","content":"hello"}]}}"#
    );
}

#[tokio::test]
async fn sends_the_output_payload_for_responses() {
    let server = server::start(vec![server::reply("{}")]);
    let guardrail = guardrail(&server.base);
    guardrail
        .evaluate(
            text_inputs(&["answer"]),
            &proxy_data(&[("hl-project-id", "project"), ("hl-requester-id", "app")]),
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("evaluated");
    let seen = server.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].headers.get("hl-project-id").map(String::as_str),
        Some("project")
    );
    assert_eq!(
        canonical(&seen[0].body),
        r#"{"metadata":{"model":"unknown","requester_id":"app"},"output":{"messages":[{"role":"user","content":"answer"}]}}"#
    );
}

#[tokio::test]
async fn sends_the_logging_model_and_headers() {
    let server = server::start(vec![server::reply("{}")]);
    let guardrail = guardrail(&server.base);
    let request_data = logging_data("gpt-4o");
    let target = GuardrailLogTarget {
        litellm_params: None,
        model_call_params: Some(BTreeMap::from([(
            "metadata".to_owned(),
            object(vec![(
                "headers",
                headers_value(&[("hl-requester-id", "log")]),
            )]),
        )])),
    };
    guardrail
        .evaluate(
            text_inputs(&["hello"]),
            &request_data,
            GuardrailInputType::Request,
            Some(&target),
        )
        .await
        .expect("evaluated");
    let seen = server.requests();
    assert_eq!(
        canonical(&seen[0].body),
        r#"{"metadata":{"model":"gpt-4o","requester_id":"log"},"input":{"messages":[{"role":"user","content":"hello"}]}}"#
    );
}

#[tokio::test]
async fn an_invalid_project_id_is_rejected_before_the_call() {
    let server = server::start(Vec::new());
    let guardrail = guardrail(&server.base);
    let result = guardrail
        .evaluate(
            text_inputs(&["hello"]),
            &proxy_data(&[("hl-project-id", "bad\nvalue")]),
            GuardrailInputType::Request,
            None,
        )
        .await;
    assert!(matches!(result, Err(EvaluationError::InvalidHeader { .. })));
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn evaluate_without_scannable_inputs_sends_nothing() {
    let server = server::start(Vec::new());
    let guardrail = guardrail(&server.base);
    let evaluated = guardrail
        .evaluate(
            GuardrailApiInputs::default(),
            &BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("evaluated");
    assert_eq!(evaluated, GuardrailApiInputs::default());
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn sends_the_bearer_token_for_saas() {
    let server = server::start(vec![server::reply("{}")]);
    let tokens = Arc::new(tokens::ScriptedTokens::new(&["token-a"]));
    let args = fixtures::args(&fixtures::saas_base(server.port));
    let guardrail = build(
        &args,
        &fixtures::environment(),
        fixtures::saas_client(server.port),
        Arc::clone(&tokens),
    );
    guardrail
        .evaluate(
            text_inputs(&["hello"]),
            &logging_data("gpt-4o"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("evaluated");
    let seen = server.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer token-a")
    );
    let fetches = tokens.fetches();
    assert_eq!(fetches.len(), 1);
    assert_eq!(
        fetches[0].url,
        "http://auth.invalid/oauth2/token?grant_type=client_credentials"
    );
    assert_eq!(fetches[0].api_id, "api-id");
    assert_eq!(fetches[0].api_key, "api-key");
}

#[tokio::test]
async fn refreshes_the_token_on_401_and_retries() {
    let server = server::start(vec![
        server::reply_with(401, &[], r#"{"error": "expired"}"#),
        server::reply(r#"{"evaluation": {"action": "Allow"}}"#),
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
            text_inputs(&["hello"]),
            &BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("evaluated");
    assert_eq!(evaluated.texts, Some(vec!["hello".to_owned()]));
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

#[tokio::test]
async fn other_status_codes_are_not_retried() {
    let server = server::start(vec![server::reply_with(403, &[], "denied")]);
    let guardrail = guardrail(&server.base);
    let result = guardrail
        .evaluate(
            text_inputs(&["hello"]),
            &BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await;
    assert!(matches!(
        result,
        Err(EvaluationError::Auth(HiddenlayerError::Status {
            status: 403,
            ..
        }))
    ));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn the_configured_timeout_stops_the_single_attempt() {
    let server = server::start(vec![server::stall(Duration::from_secs(1))]);
    let args = EvaluationArgs {
        timeout: Some(0.5),
        ..fixtures::args(&server.base)
    };
    let guardrail = build(
        &args,
        &fixtures::environment(),
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
    );
    let result = guardrail
        .evaluate(
            text_inputs(&["hello"]),
            &BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await;
    let Err(EvaluationError::Auth(HiddenlayerError::Transport { source })) = result else {
        panic!("a detection call that timed out");
    };
    assert!(source.is_timeout(), "{source}");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_non_json_answer_is_a_decode_error() {
    let server = server::start(vec![server::reply("not json")]);
    let guardrail = guardrail(&server.base);
    let result = guardrail
        .evaluate(
            text_inputs(&["hello"]),
            &BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await;
    assert!(matches!(result, Err(EvaluationError::Decode { .. })));
}

#[tokio::test]
async fn the_api_base_can_come_from_the_environment() {
    let server = server::start(vec![server::reply("{}")]);
    let mut args = fixtures::args("http://unused.invalid");
    args.api_base = None;
    let environment = BTreeMap::from([("HIDDENLAYER_API_BASE".to_owned(), server.base.clone())]);
    let guardrail = build(
        &args,
        &environment,
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
    );
    guardrail
        .evaluate(
            text_inputs(&["hello"]),
            &BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("evaluated");
    assert_eq!(server.requests().len(), 1);
}
