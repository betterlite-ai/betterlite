use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;

use super::support::{
    GUARDRAIL_NAME, SAFE, UNSAFE, USER_REQUEST, field, guard_at, guard_named, guard_with_block,
    inputs, messages_of, mock, request_data, seen, text,
};

const GROUNDING_UNSAFE: &str = r#"{"decision":"UNSAFE","trace_id":"trace-3","xecguard_result":{"violated_rules_list":["RuleA"],"rationale":"not grounded"}}"#;
const RESPONSE_REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"question"}],"response":{"choices":[{"message":{"role":"assistant","content":"answer"}}]}}"#;
const MULTI_TURN_GROUNDING_REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"question"},{"role":"assistant","content":"answer"},{"role":"user","content":"follow-up"}],"metadata":{"xecguard_grounding_documents":[{"document_id":"doc-1","context":"the context"}]}}"#;
const GROUNDING_REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"question"}],"response":{"choices":[{"message":{"role":"assistant","content":"answer"}}]},"metadata":{"xecguard_grounding_documents":[{"document_id":"doc-1","context":"the context"}]}}"#;

pub(super) async fn safe_request_scan() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(USER_REQUEST);
    let original = inputs(&["hello"]);
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a pass");
    assert_eq!(returned, original);
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    let messages = messages_of(&seen[0].body);
    assert_eq!(messages.len(), 1);
    assert_eq!(text(field(&messages[0], "role")), "user");
    assert_eq!(text(field(&messages[0], "content")), "hello");
}

pub(super) async fn unsafe_request_scan() {
    let mock = mock((200, UNSAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(USER_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("an UNSAFE scan blocks the request");
    let (status, detail) = blocked_object(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error").map_or("", text),
        "Blocked by XecGuard: policies=[HarmfulContent] trace_id=trace-2 rationale=forbidden"
    );
    assert_eq!(
        detail.get("guardrail_name").map_or("", text),
        GUARDRAIL_NAME
    );
    let response = detail.get("xecguard_response").expect("the scan result");
    assert_eq!(text(field(response, "decision")), "UNSAFE");
    assert_eq!(text(field(response, "trace_id")), "trace-2");
}

pub(super) async fn safe_response_scan() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(RESPONSE_REQUEST);
    let original = inputs(&["answer"]);
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("a pass");
    assert_eq!(returned, original);
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/xecguard/v1/scan");
    assert_eq!(text(field(&seen[0].body, "scan_type")), "response");
    let messages = messages_of(&seen[0].body);
    assert_eq!(messages.len(), 2);
    assert_eq!(text(field(&messages[1], "role")), "assistant");
    assert_eq!(text(field(&messages[1], "content")), "answer");
}

pub(super) async fn unsafe_response_scan() {
    let mock = mock((200, UNSAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(GROUNDING_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["answer"]),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect_err("an UNSAFE response scan blocks the response");
    let (status, detail) = blocked_object(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error").map_or("", text),
        "Blocked by XecGuard: policies=[HarmfulContent] trace_id=trace-2 rationale=forbidden"
    );
    assert_eq!(seen(&mock).len(), 1);
}

pub(super) async fn grounding_block() {
    let mock = mock((200, SAFE), (200, GROUNDING_UNSAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(GROUNDING_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["answer"]),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect_err("an UNSAFE grounding check blocks the response");
    let (status, detail) = blocked_object(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error").map_or("", text),
        "Blocked by XecGuard grounding: rules=[RuleA] trace_id=trace-3 rationale=not grounded"
    );
    assert_eq!(
        detail.get("guardrail_name").map_or("", text),
        GUARDRAIL_NAME
    );
    let response = detail
        .get("xecguard_response")
        .expect("the grounding result");
    assert_eq!(text(field(response, "decision")), "UNSAFE");
    assert_eq!(text(field(response, "trace_id")), "trace-3");
    let seen = seen(&mock);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].path, "/xecguard/v1/grounding");
    assert_eq!(text(field(&seen[1].body, "prompt")), "question");
    assert_eq!(text(field(&seen[1].body, "response")), "answer");
}

pub(super) async fn block_on_error_true() {
    let mock = mock((500, "{}"), (200, SAFE));
    let guard = guard_with_block(&mock.base, true);
    let mut data = request_data(USER_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("block_on_error blocks an unreachable API");
    let (status, detail) = blocked_object(&failure);
    assert_eq!(status, 400);
    let message = detail.get("error").map_or("", text);
    assert!(
        message.starts_with("XecGuard API unreachable (block_on_error=True): "),
        "{message}"
    );
    assert!(message.contains("500"), "{message}");
    assert_eq!(
        detail.get("guardrail_name").map_or("", text),
        GUARDRAIL_NAME
    );
    assert!(detail.get("xecguard_response").is_none());
}

pub(super) async fn block_on_error_false() {
    let mock = mock((500, "{}"), (200, SAFE));
    let guard = guard_with_block(&mock.base, false);
    let mut data = request_data(USER_REQUEST);
    let original = inputs(&["hello"]);
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a fail-open pass");
    assert_eq!(returned, original);
    assert_eq!(seen(&mock).len(), 1);
}

pub(super) async fn empty_history() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let original = GuardrailApiInputs::default();
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a pass with no messages");
    assert_eq!(returned, original);
    assert!(seen(&mock).is_empty());
}

pub(super) async fn default_guardrail_name() {
    let mock = mock((200, UNSAFE), (200, SAFE));
    assert_eq!(blocked_name(&mock.base, None).await, "xecguard");
    assert_eq!(blocked_name(&mock.base, Some("")).await, "xecguard");
    assert_eq!(seen(&mock).len(), 2);
}

async fn blocked_name(api_base: &str, name: Option<&str>) -> String {
    let guard = guard_named(api_base, name);
    let mut data = request_data(USER_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("an UNSAFE scan blocks the request");
    let (_status, detail) = blocked_object(&failure);
    detail.get("guardrail_name").map_or("", text).to_owned()
}

pub(super) async fn grounding_failure_fails_open() {
    let mock = mock((200, SAFE), (500, "{}"));
    let guard = guard_with_block(&mock.base, false);
    let mut data = request_data(GROUNDING_REQUEST);
    let original = inputs(&["answer"]);
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("a fail-open grounding pass");
    assert_eq!(returned, original);
    let seen = seen(&mock);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].path, "/xecguard/v1/grounding");
}

pub(super) async fn grounding_failure_blocks() {
    let mock = mock((200, SAFE), (500, "{}"));
    let guard = guard_with_block(&mock.base, true);
    let mut data = request_data(GROUNDING_REQUEST);
    let failure = guard
        .apply_guardrail(
            inputs(&["answer"]),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect_err("block_on_error blocks an unreachable grounding API");
    let (status, detail) = blocked_object(&failure);
    assert_eq!(status, 400);
    let message = detail.get("error").map_or("", text);
    assert!(
        message.starts_with("XecGuard API unreachable (block_on_error=True): "),
        "{message}"
    );
    assert_eq!(
        detail.get("guardrail_name").map_or("", text),
        GUARDRAIL_NAME
    );
    assert!(detail.get("xecguard_response").is_none());
    assert_eq!(seen(&mock).len(), 2);
}

pub(super) async fn grounding_passes() {
    let mock = mock((200, SAFE), (200, SAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(GROUNDING_REQUEST);
    let original = inputs(&["answer"]);
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("a grounded pass");
    assert_eq!(returned, original);
    let seen = seen(&mock);
    assert_eq!(seen.len(), 2);
    assert_eq!(text(field(&seen[1].body, "strictness")), "BALANCED");
}

pub(super) async fn request_ignores_grounding_documents() {
    let mock = mock((200, SAFE), (200, GROUNDING_UNSAFE));
    let guard = guard_at(&mock.base);
    let mut data = request_data(MULTI_TURN_GROUNDING_REQUEST);
    let original = inputs(&["follow-up"]);
    let returned = guard
        .apply_guardrail(
            original.clone(),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a request is never grounding checked");
    assert_eq!(returned, original);
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/xecguard/v1/scan");
    assert_eq!(text(field(&seen[0].body, "scan_type")), "input");
    assert_eq!(messages_of(&seen[0].body).len(), 3);
}

fn blocked_object(failure: &GuardrailFailure) -> (u16, &IndexMap<String, GuardrailValue>) {
    match failure {
        GuardrailFailure::HttpObject { status, detail } => (*status, detail),
        other => panic!("an object failure: {other:?}"),
    }
}
