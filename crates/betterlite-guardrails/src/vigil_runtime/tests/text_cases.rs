#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::inputs::GuardrailLogTarget;

use super::cases::Scenario;
use super::fixtures::{
    BACKEND_CLOSED, HARMFUL, UNRECOGNIZED, allowed, blocked, header_args, message,
    missing_decision, non_object_answer, other, raised, request, response, rewritten, sanitized,
    tool_call, tool_param, unavailable, unknown_decision,
};

fn add_media(case: &mut Scenario) {
    case.inputs.images = Some(vec!["https://images.example/one".to_owned()]);
    case.inputs.tools = Some(vec![tool_param()]);
    case.inputs.structured_messages = Some(vec![message()]);
    case.inputs.model = Some("gpt-4o".to_owned());
    case.inputs.stream_holdback_chars = Some(vec![7]);
}

pub(crate) fn no_texts_skip_the_guardrail() -> Scenario {
    request(None, vec![allowed()])
}

pub(crate) fn whitespace_only_texts_skip_the_guardrail() -> Scenario {
    request(
        Some(vec![" ", "\n\t", "", "\u{1c}", "\u{1f}"]),
        vec![allowed()],
    )
}

pub(crate) fn blank_text_positions_are_kept_around_a_rewrite() -> Scenario {
    let mut case = request(Some(vec![" ", "secret", ""]), vec![sanitized("clean")]);
    case.analyzed = vec!["secret"];
    case.expected = rewritten(&[" ", "clean", ""], None);
    case
}

pub(crate) fn request_tools_and_tool_calls_are_not_scanned() -> Scenario {
    let mut case = request(Some(vec!["hello"]), vec![allowed()]);
    case.inputs.tools = Some(vec![tool_param()]);
    case.inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    case.analyzed = vec!["hello"];
    case
}

pub(crate) fn a_blocked_text_stops_before_later_texts_and_tools() -> Scenario {
    let mut case = response(Some(vec!["bad", "later"]), &[("call_1", "a1")], Vec::new());
    case.responses = vec![blocked(HARMFUL)];
    case.analyzed = vec!["bad"];
    case.expected = raised(HARMFUL, 400, true);
    case
}

pub(crate) fn a_text_stage_backend_failure_keeps_the_original_tool_calls() -> Scenario {
    let mut case = response(Some(vec!["one", "two"]), &[("call_1", "a1")], Vec::new());
    case.responses = vec![sanitized("clean"), unavailable()];
    case.fail_open(vec!["one", "two"]);
    case.expected = rewritten(&["clean", "two"], None);
    case
}

pub(crate) fn a_missing_decision_fails_open_with_the_partial_texts() -> Scenario {
    let mut case = response(
        Some(vec!["secret", "second"]),
        &[("call_1", "a1")],
        Vec::new(),
    );
    case.responses = vec![sanitized("clean"), missing_decision()];
    case.fail_open(vec!["secret", "second"]);
    case.expected = rewritten(&["clean", "second"], None);
    case
}

pub(crate) fn an_unknown_decision_fails_closed() -> Scenario {
    let mut case = request(Some(vec!["secret"]), Vec::new());
    case.responses = vec![unknown_decision()];
    case.analyzed = vec!["secret"];
    case.expected = raised(UNRECOGNIZED, 400, false);
    case
}

pub(crate) fn an_unknown_decision_fails_open() -> Scenario {
    let mut case = request(Some(vec!["secret"]), Vec::new());
    case.responses = vec![unknown_decision()];
    case.fail_open(vec!["secret"]);
    case
}

pub(crate) fn metadata_and_call_id_reach_every_analysis() -> Scenario {
    let mut nested = BTreeMap::new();
    nested.insert("user".to_owned(), GuardrailValue::Text("alice".to_owned()));
    nested.insert(
        "litellm_call_id".to_owned(),
        GuardrailValue::Text("call-request".to_owned()),
    );
    let mut request_data = BTreeMap::new();
    request_data.insert(
        "model".to_owned(),
        GuardrailValue::Text("gpt-4o".to_owned()),
    );
    request_data.insert("metadata".to_owned(), GuardrailValue::Object(nested));
    let mut litellm_params = BTreeMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        GuardrailValue::Text("call-litellm".to_owned()),
    );
    let mut model_call_params = BTreeMap::new();
    model_call_params.insert(
        "litellm_call_id".to_owned(),
        GuardrailValue::Text("call-model".to_owned()),
    );
    let mut case = request(Some(vec!["one", "two"]), Vec::new());
    case.responses = vec![allowed(), allowed()];
    case.analyzed = vec!["one", "two"];
    case.request_data = request_data;
    case.logging = Some(GuardrailLogTarget {
        litellm_params: Some(litellm_params),
        model_call_params: Some(model_call_params),
    });
    case.metadata = vec![
        ("model", "gpt-4o"),
        ("user", "alice"),
        ("litellm_call_id", "call-model"),
    ];
    case
}

pub(crate) fn an_empty_call_id_does_not_shadow_a_later_one() -> Scenario {
    let mut litellm_params = BTreeMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        GuardrailValue::Text("call-litellm".to_owned()),
    );
    let mut model_call_params = BTreeMap::new();
    model_call_params.insert(
        "litellm_call_id".to_owned(),
        GuardrailValue::Text(String::new()),
    );
    let mut case = request(Some(vec!["hello"]), Vec::new());
    case.responses = vec![allowed()];
    case.analyzed = vec!["hello"];
    case.logging = Some(GuardrailLogTarget {
        litellm_params: Some(litellm_params),
        model_call_params: Some(model_call_params),
    });
    case.metadata = vec![("litellm_call_id", "call-litellm")];
    case
}

pub(crate) fn an_unchanged_full_record_is_preserved() -> Scenario {
    let mut case = response(Some(vec!["hello"]), &[], Vec::new());
    case.inputs.tool_calls = Some(vec![tool_call("call_1", r#"{"argument": 1}"#)]);
    add_media(&mut case);
    case.responses = vec![allowed(), allowed()];
    case.analyzed = vec!["hello", r#"{"argument": 1}"#];
    case
}

pub(crate) fn a_sanitized_text_drops_stale_structured_messages() -> Scenario {
    let mut case = request(Some(vec!["secret"]), Vec::new());
    add_media(&mut case);
    case.responses = vec![sanitized("clean")];
    case.analyzed = vec!["secret"];
    case.expected = rewritten(&["clean"], None);
    case
}

pub(crate) fn an_invalid_header_fails_open() -> Scenario {
    let mut case = request(Some(vec!["hello"]), Vec::new());
    case.args = header_args("bad\nkey", "fail_open");
    case
}

pub(crate) fn a_non_ascii_api_key_is_not_swallowed_by_fail_open() -> Scenario {
    let mut case = request(Some(vec!["hello"]), Vec::new());
    case.args = header_args("sk-vigil-é", "fail_open");
    case.expected = other("non-ASCII");
    case
}

pub(crate) fn a_non_ascii_api_key_is_not_a_backend_failure_when_closed() -> Scenario {
    let mut case = request(Some(vec!["hello"]), Vec::new());
    case.args = header_args("sk-vigil-é", "fail_closed");
    case.expected = other("non-ASCII");
    case
}

pub(crate) fn an_invalid_header_fails_closed() -> Scenario {
    let mut case = request(Some(vec!["hello"]), Vec::new());
    case.args = header_args("bad\nkey", "fail_closed");
    case.expected = raised(BACKEND_CLOSED, 400, false);
    case
}

pub(crate) fn a_non_object_answer_is_a_backend_failure() -> Scenario {
    let mut case = request(Some(vec!["secret"]), vec![non_object_answer()]);
    case.analyzed = vec!["secret"];
    case.expected = raised(BACKEND_CLOSED, 400, false);
    case
}

pub(crate) fn a_raised_failure_carries_the_guardrail_name() -> Scenario {
    let mut case = request(Some(vec!["bad"]), vec![blocked(HARMFUL)]);
    case.args.name = "vigil-check".to_owned();
    case.analyzed = vec!["bad"];
    case.expected = raised(HARMFUL, 400, true);
    case
}

pub(crate) fn an_invalid_tool_call_is_not_a_backend_failure() -> Scenario {
    let mut case = response(None, &[], Vec::new());
    case.inputs.tool_calls = Some(vec![GuardrailValue::Text("nope".to_owned())]);
    case.responses = vec![allowed()];
    case.fail_open(Vec::new());
    case.expected = other("index 0");
    case
}
