#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailLogTarget;

use super::cases::{Case, Expected};
use super::fixtures::{
    allowed, args, blocked, message, missing_decision, sanitized, scan_case, texts_inputs,
    tool_call, tool_param, unavailable, unknown_decision,
};

pub(crate) fn no_texts_skip_the_guardrail() -> Case {
    let scenario = scan_case(texts_inputs(None), vec![allowed()]);
    Case::Scan(Box::new(scenario))
}

pub(crate) fn whitespace_only_texts_skip_the_guardrail() -> Case {
    let scenario = scan_case(texts_inputs(Some(vec![" ", "\n\t", ""])), vec![allowed()]);
    Case::Scan(Box::new(scenario))
}

pub(crate) fn blank_text_positions_are_kept_around_a_rewrite() -> Case {
    let mut scenario = scan_case(
        texts_inputs(Some(vec![" ", "secret", ""])),
        vec![sanitized("clean")],
    );
    scenario.analyzed = vec!["secret"];
    scenario.expected = Expected::Rewritten {
        texts: vec![" ".to_owned(), "clean".to_owned(), String::new()],
        tool_calls: None,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn request_tools_and_tool_calls_are_not_scanned() -> Case {
    let mut inputs = texts_inputs(Some(vec!["hello"]));
    inputs.tools = Some(vec![tool_param()]);
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    let mut scenario = scan_case(inputs, vec![allowed()]);
    scenario.analyzed = vec!["hello"];
    Case::Scan(Box::new(scenario))
}

pub(crate) fn a_blocked_text_stops_before_later_texts_and_tools() -> Case {
    let mut inputs = texts_inputs(Some(vec!["bad", "later"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    let mut scenario = scan_case(inputs, vec![blocked("Harmful content")]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.analyzed = vec!["bad"];
    scenario.expected = Expected::Raised {
        message: "Harmful content".to_owned(),
        status_code: 400,
        blocked_content: true,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn a_text_stage_backend_failure_keeps_the_original_tool_calls() -> Case {
    let mut inputs = texts_inputs(Some(vec!["first", "second"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    let mut scenario = scan_case(inputs, vec![sanitized("clean"), unavailable()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.args.unreachable_fallback = "fail_open".to_owned();
    scenario.analyzed = vec!["first", "second"];
    scenario.expected = Expected::Rewritten {
        texts: vec!["clean".to_owned(), "second".to_owned()],
        tool_calls: None,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn a_missing_decision_fails_open_with_the_partial_texts() -> Case {
    let mut inputs = texts_inputs(Some(vec!["secret", "second"]));
    inputs.tool_calls = Some(vec![tool_call("call_1", "a1")]);
    let mut scenario = scan_case(inputs, vec![sanitized("clean"), missing_decision()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.args.unreachable_fallback = "fail_open".to_owned();
    scenario.analyzed = vec!["secret", "second"];
    scenario.expected = Expected::Rewritten {
        texts: vec!["clean".to_owned(), "second".to_owned()],
        tool_calls: None,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn an_unknown_decision_fails_closed() -> Case {
    let mut scenario = scan_case(texts_inputs(Some(vec!["secret"])), vec![unknown_decision()]);
    scenario.analyzed = vec!["secret"];
    scenario.expected = Expected::Raised {
        message: "Vigil Guard returned an unrecognized decision.".to_owned(),
        status_code: 400,
        blocked_content: false,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn an_unknown_decision_fails_open() -> Case {
    let mut scenario = scan_case(texts_inputs(Some(vec!["secret"])), vec![unknown_decision()]);
    scenario.args.unreachable_fallback = "fail_open".to_owned();
    scenario.analyzed = vec!["secret"];
    Case::Scan(Box::new(scenario))
}

pub(crate) fn metadata_and_call_id_reach_every_analysis() -> Case {
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
    let mut scenario = scan_case(
        texts_inputs(Some(vec!["one", "two"])),
        vec![allowed(), allowed()],
    );
    scenario.analyzed = vec!["one", "two"];
    scenario.request_data = request_data;
    scenario.logging = Some(GuardrailLogTarget {
        litellm_params: Some(litellm_params),
        model_call_params: Some(model_call_params),
    });
    scenario.metadata = vec![
        ("model", "gpt-4o"),
        ("user", "alice"),
        ("litellm_call_id", "call-model"),
    ];
    Case::Scan(Box::new(scenario))
}

pub(crate) fn an_empty_call_id_does_not_shadow_a_later_one() -> Case {
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
    let mut scenario = scan_case(texts_inputs(Some(vec!["hello"])), vec![allowed()]);
    scenario.analyzed = vec!["hello"];
    scenario.logging = Some(GuardrailLogTarget {
        litellm_params: Some(litellm_params),
        model_call_params: Some(model_call_params),
    });
    scenario.metadata = vec![("litellm_call_id", "call-litellm")];
    Case::Scan(Box::new(scenario))
}

pub(crate) fn an_unchanged_full_record_is_preserved() -> Case {
    let mut inputs = texts_inputs(Some(vec!["hello"]));
    inputs.images = Some(vec!["https://images.example/one".to_owned()]);
    inputs.tools = Some(vec![tool_param()]);
    inputs.tool_calls = Some(vec![tool_call("call_1", r#"{"argument": 1}"#)]);
    inputs.structured_messages = Some(vec![message()]);
    inputs.model = Some("gpt-4o".to_owned());
    inputs.stream_holdback_chars = Some(vec![7]);
    let mut scenario = scan_case(inputs, vec![allowed(), allowed()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.analyzed = vec!["hello", r#"{"argument": 1}"#];
    Case::Scan(Box::new(scenario))
}

pub(crate) fn a_sanitized_text_drops_stale_structured_messages() -> Case {
    let mut inputs = texts_inputs(Some(vec!["secret"]));
    inputs.images = Some(vec!["https://images.example/one".to_owned()]);
    inputs.tools = Some(vec![tool_param()]);
    inputs.structured_messages = Some(vec![message()]);
    inputs.model = Some("gpt-4o".to_owned());
    inputs.stream_holdback_chars = Some(vec![7]);
    let mut scenario = scan_case(inputs, vec![sanitized("clean")]);
    scenario.analyzed = vec!["secret"];
    scenario.expected = Expected::Rewritten {
        texts: vec!["clean".to_owned()],
        tool_calls: None,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn an_invalid_header_fails_open() -> Case {
    let mut scenario = scan_case(texts_inputs(Some(vec!["hello"])), Vec::new());
    scenario.args = args(
        Some("https://vigil.example"),
        Some("bad\nkey"),
        "fail_open",
        None,
    );
    Case::Scan(Box::new(scenario))
}

pub(crate) fn an_invalid_header_fails_closed() -> Case {
    let mut scenario = scan_case(texts_inputs(Some(vec!["hello"])), Vec::new());
    scenario.args = args(
        Some("https://vigil.example"),
        Some("bad\nkey"),
        "fail_closed",
        None,
    );
    scenario.expected = Expected::Raised {
        message: "Vigil Guard backend unreachable; request blocked by fail_closed policy."
            .to_owned(),
        status_code: 400,
        blocked_content: false,
    };
    Case::Scan(Box::new(scenario))
}

pub(crate) fn an_invalid_tool_call_is_not_a_backend_failure() -> Case {
    let mut inputs = texts_inputs(None);
    inputs.tool_calls = Some(vec![GuardrailValue::Text("nope".to_owned())]);
    let mut scenario = scan_case(inputs, vec![allowed()]);
    scenario.input_type = GuardrailInputType::Response;
    scenario.args = args(
        Some("https://vigil.example"),
        Some("sk-vigil-test"),
        "fail_open",
        None,
    );
    scenario.expected = Expected::Other("index 0".to_owned());
    Case::Scan(Box::new(scenario))
}
