#![cfg(test)]

use crate::generic_guardrail_contracts::GuardrailInputType;

use super::cases::{Case, Expected};
use super::fixtures::{
    allowed, message, sanitized, scan_case, texts_inputs, tool_call, unavailable, unknown_decision,
};

pub(crate) fn texts_are_scanned_before_response_tool_arguments() -> Case {
    let mut inputs = texts_inputs(Some(vec!["secret"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    inputs.structured_messages = Some(vec![message()]);
    let mut scenario = scan_case(inputs, vec![sanitized("clean"), sanitized("b1")]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.analyzed = vec!["secret", "a1"];
    scenario.expected = Expected::Rewritten {
        texts: vec!["clean".to_owned()],
        tool_calls: Some(vec![tool_call("call_1", "b1")]),
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn an_allowed_tool_argument_leaves_tool_calls_out() -> Case {
    let mut inputs = texts_inputs(Some(vec!["secret"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    let mut scenario = scan_case(inputs, vec![sanitized("clean"), allowed()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.analyzed = vec!["secret", "a1"];
    scenario.expected = Expected::Rewritten {
        texts: vec!["clean".to_owned()],
        tool_calls: None,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn a_tool_stage_backend_failure_keeps_the_earlier_rewrites() -> Case {
    let mut inputs = texts_inputs(Some(vec!["first"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1"), tool_call("call_2", "a2")]);
    let mut scenario = scan_case(inputs, vec![allowed(), sanitized("b1"), unavailable()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.args.unreachable_fallback = "fail_open".to_owned();
    scenario.analyzed = vec!["first", "a1", "a2"];
    scenario.expected = Expected::Rewritten {
        texts: vec!["first".to_owned()],
        tool_calls: Some(vec![tool_call("call_1", "b1"), tool_call("call_2", "a2")]),
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn a_tool_stage_backend_failure_fails_closed() -> Case {
    let mut inputs = texts_inputs(Some(vec!["first"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    let mut scenario = scan_case(inputs, vec![allowed(), unavailable()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.analyzed = vec!["first", "a1"];
    scenario.expected = Expected::Raised {
        message: "Vigil Guard backend unreachable; request blocked by fail_closed policy."
            .to_owned(),
        status_code: 400,
        blocked_content: false,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn blank_tool_arguments_are_not_scanned() -> Case {
    let mut inputs = texts_inputs(Some(vec!["hello"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "  "), tool_call("call_2", "a2")]);
    let mut scenario = scan_case(inputs, vec![allowed(), allowed()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.analyzed = vec!["hello", "a2"];
    Case::Scan(Box::new(scenario))
}

pub(crate) fn a_tool_stage_unknown_decision_fails_open() -> Case {
    let mut inputs = texts_inputs(Some(vec!["hello"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    let mut scenario = scan_case(inputs, vec![allowed(), unknown_decision()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.args.unreachable_fallback = "fail_open".to_owned();
    scenario.analyzed = vec!["hello", "a1"];
    Case::Scan(Box::new(scenario))
}

pub(crate) fn a_tool_stage_unknown_decision_fails_closed() -> Case {
    let mut inputs = texts_inputs(Some(vec!["hello"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    let mut scenario = scan_case(inputs, vec![allowed(), unknown_decision()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.analyzed = vec!["hello", "a1"];
    scenario.expected = Expected::Raised {
        message: "Vigil Guard returned an unrecognized decision.".to_owned(),
        status_code: 400,
        blocked_content: false,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn a_tool_only_rewrite_returns_empty_texts() -> Case {
    let mut inputs = texts_inputs(None);
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    let mut scenario = scan_case(inputs, vec![sanitized("b1")]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.analyzed = vec!["a1"];
    scenario.expected = Expected::Rewritten {
        texts: Vec::new(),
        tool_calls: Some(vec![tool_call("call_1", "b1")]),
    };
    Case::Scan(Box::new(scenario))
}
