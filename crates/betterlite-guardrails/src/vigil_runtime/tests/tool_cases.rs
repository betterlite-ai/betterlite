#![cfg(test)]

use super::cases::Scenario;
use super::fixtures::{
    BACKEND_CLOSED, UNRECOGNIZED, allowed, message, raised, response, rewritten, sanitized,
    tool_call, unavailable, unknown_decision,
};

pub(crate) fn texts_are_scanned_before_response_tool_arguments() -> Scenario {
    let mut case = response(Some(vec!["secret"]), &[("call_1", "a1")], Vec::new());
    case.inputs.structured_messages = Some(vec![message()]);
    case.responses = vec![sanitized("clean"), sanitized("b1")];
    case.analyzed = vec!["secret", "a1"];
    case.expected = rewritten(vec!["clean"], Some(vec![tool_call("call_1", "b1")]));
    case
}

pub(crate) fn an_allowed_tool_argument_leaves_tool_calls_out() -> Scenario {
    let mut case = response(Some(vec!["secret"]), &[("call_1", "a1")], Vec::new());
    case.responses = vec![sanitized("clean"), allowed()];
    case.analyzed = vec!["secret", "a1"];
    case.expected = rewritten(vec!["clean"], None);
    case
}

pub(crate) fn a_tool_stage_backend_failure_keeps_the_earlier_rewrites() -> Scenario {
    let mut case = response(Some(vec!["first"]), &[], Vec::new());
    case.inputs.tool_calls = Some(vec![tool_call("call_1", "a1"), tool_call("call_2", "a2")]);
    case.responses = vec![allowed(), sanitized("b1"), unavailable()];
    case.fail_open(vec!["first", "a1", "a2"]);
    case.expected = rewritten(
        vec!["first"],
        Some(vec![tool_call("call_1", "b1"), tool_call("call_2", "a2")]),
    );
    case
}

pub(crate) fn a_tool_stage_backend_failure_fails_closed() -> Scenario {
    let mut case = response(Some(vec!["first"]), &[("call_1", "a1")], Vec::new());
    case.responses = vec![allowed(), unavailable()];
    case.analyzed = vec!["first", "a1"];
    case.expected = raised(BACKEND_CLOSED, 400, false);
    case
}

pub(crate) fn blank_tool_arguments_are_not_scanned() -> Scenario {
    let mut case = response(Some(vec!["hello"]), &[], Vec::new());
    case.inputs.tool_calls = Some(vec![tool_call("call_1", "  "), tool_call("call_2", "a2")]);
    case.responses = vec![allowed(), allowed()];
    case.analyzed = vec!["hello", "a2"];
    case
}

pub(crate) fn a_tool_stage_unknown_decision_fails_open() -> Scenario {
    let mut case = response(Some(vec!["hello"]), &[("call_1", "a1")], Vec::new());
    case.responses = vec![allowed(), unknown_decision()];
    case.fail_open(vec!["hello", "a1"]);
    case
}

pub(crate) fn a_tool_stage_unknown_decision_fails_closed() -> Scenario {
    let mut case = response(Some(vec!["hello"]), &[("call_1", "a1")], Vec::new());
    case.responses = vec![allowed(), unknown_decision()];
    case.analyzed = vec!["hello", "a1"];
    case.expected = raised(UNRECOGNIZED, 400, false);
    case
}

pub(crate) fn a_tool_only_rewrite_returns_empty_texts() -> Scenario {
    let mut case = response(None, &[("call_1", "a1")], Vec::new());
    case.responses = vec![sanitized("b1")];
    case.analyzed = vec!["a1"];
    case.expected = rewritten(Vec::new(), Some(vec![tool_call("call_1", "b1")]));
    case
}
