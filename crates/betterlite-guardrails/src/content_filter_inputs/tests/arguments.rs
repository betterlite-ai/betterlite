#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ContentFilterAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::content_filter_inputs::arguments::{
    ArgumentFilter, filter_argument_value, scan_tool_call_arguments,
};
use crate::content_filter_inputs::error::ArgumentFilterError;
use crate::guardrail_base::failure::GuardrailFailure;

use super::{Fixture, pattern_names, regex_pattern};

fn text(arguments: &str) -> GuardrailValue {
    GuardrailValue::Text(arguments.to_owned())
}

fn tool_call(arguments: GuardrailValue) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        ("id".to_owned(), text("call-1")),
        ("type".to_owned(), text("function")),
        (
            "function".to_owned(),
            GuardrailValue::Object(BTreeMap::from([
                ("name".to_owned(), text("lookup")),
                ("arguments".to_owned(), arguments),
            ])),
        ),
    ]))
}

fn arguments_of(call: &GuardrailValue) -> Option<&str> {
    let GuardrailValue::Object(entries) = call else {
        return None;
    };
    let GuardrailValue::Object(function) = entries.get("function")? else {
        return None;
    };
    let GuardrailValue::Text(arguments) = function.get("arguments")? else {
        return None;
    };
    Some(arguments.as_str())
}

#[test]
fn tool_call_argument_filtering() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "secret",
        "secret",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut calls = vec![
        tool_call(text(
            r#"{"query": "a secret value", "n": 3, "ok": true, "gone": null}"#,
        )),
        tool_call(text("{ \"plain\": 1 }")),
    ];
    let mut detections = Vec::new();
    scan_tool_call_arguments(&filter, &mut calls, &mut detections)
        .expect("the tool call arguments filter");
    assert_eq!(
        arguments_of(&calls[0]),
        Some(r#"{"gone":null,"n":3,"ok":true,"query":"a [SECRET_REDACTED] value"}"#)
    );
    assert_eq!(arguments_of(&calls[1]), Some("{ \"plain\": 1 }"));
    assert_eq!(pattern_names(&detections), ["secret"]);
}

#[test]
fn tool_call_malformed_json_falls_back_to_text() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "secret",
        "secret",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut calls = vec![tool_call(text("not json secret"))];
    let mut detections = Vec::new();
    scan_tool_call_arguments(&filter, &mut calls, &mut detections)
        .expect("the malformed arguments filter as text");
    assert_eq!(arguments_of(&calls[0]), Some("not json [SECRET_REDACTED]"));
    assert_eq!(pattern_names(&detections), ["secret"]);
}

#[test]
fn tool_call_nan_arguments_fall_back_to_text() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "special",
        "NaN",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut calls = vec![tool_call(text(r#"{"n": NaN}"#))];
    let mut detections = Vec::new();
    scan_tool_call_arguments(&filter, &mut calls, &mut detections)
        .expect("non-standard JSON filters as text");
    assert_eq!(
        arguments_of(&calls[0]),
        Some(r#"{"n": [SPECIAL_REDACTED]}"#)
    );
    assert_eq!(pattern_names(&detections), ["special"]);
}

#[test]
fn tool_call_text_fallback_blocks_on_a_blocking_pattern() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "leak",
        "leak",
        ContentFilterAction::Block,
    )]);
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut calls = vec![tool_call(text("not json leak"))];
    let mut detections = Vec::new();
    let error = scan_tool_call_arguments(&filter, &mut calls, &mut detections).unwrap_err();
    assert_eq!(error.to_string(), "the content filter blocked the text");
    let GuardrailFailure::HttpObject { status, detail } = error.into_failure() else {
        panic!("a blocked argument must raise the HTTP detail");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Content blocked: leak pattern detected"))
    );
}

#[test]
fn tool_call_non_object_json_is_filtered_as_text() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "secret",
        "secret",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut calls = vec![tool_call(text(r#""a secret value""#))];
    let mut detections = Vec::new();
    scan_tool_call_arguments(&filter, &mut calls, &mut detections)
        .expect("a JSON string argument filters as text");
    assert_eq!(
        arguments_of(&calls[0]),
        Some(r#""a [SECRET_REDACTED] value""#)
    );
}

#[test]
fn tool_call_skips_calls_without_usable_arguments() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "secret",
        "secret",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut calls = vec![
        GuardrailValue::Object(BTreeMap::from([(
            "function".to_owned(),
            text("not an object"),
        )])),
        tool_call(text("   ")),
        tool_call(GuardrailValue::Object(BTreeMap::new())),
        text("not a tool call"),
    ];
    let original = calls.clone();
    let mut detections = Vec::new();
    scan_tool_call_arguments(&filter, &mut calls, &mut detections)
        .expect("calls without arguments are skipped");
    assert_eq!(calls, original);
    assert!(detections.is_empty());
}

#[test]
fn tool_call_numbers_are_checked_for_masking() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "digits",
        r"\d",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut calls = vec![tool_call(text(r#"{"n": 4111}"#))];
    let mut detections = Vec::new();
    let error = scan_tool_call_arguments(&filter, &mut calls, &mut detections).unwrap_err();
    let GuardrailFailure::HttpObject { status, detail } = error.into_failure() else {
        panic!("a masked number label must raise the HTTP detail");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text(
            "Content blocked: tool call argument matched a masking rule on a non-rewritable field"
        ))
    );
}

#[test]
fn tool_call_object_keys_are_checked_for_masking() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "secret",
        "secret",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut calls = vec![tool_call(text(r#"{"secret": "plain"}"#))];
    let mut detections = Vec::new();
    let error = scan_tool_call_arguments(&filter, &mut calls, &mut detections).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Content blocked: tool call argument matched a masking rule on a non-rewritable field"
    );
}

#[test]
fn tool_call_booleans_and_nulls_pass_through() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "secret",
        "secret",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let arguments = r#"{ "a": true, "b": null, "n": 4111 }"#;
    let mut calls = vec![tool_call(text(arguments))];
    let mut detections = Vec::new();
    scan_tool_call_arguments(&filter, &mut calls, &mut detections)
        .expect("leaves that cannot be rewritten pass through");
    assert_eq!(arguments_of(&calls[0]), Some(arguments));
    assert!(detections.is_empty());
}

#[test]
fn tool_call_depth_limit_starts_at_zero() {
    let fixture = Fixture::empty();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 1,
    };
    let mut detections = Vec::new();
    let shallow = GuardrailValue::Object(BTreeMap::from([("a".to_owned(), text("plain"))]));
    filter_argument_value(&filter, &shallow, &mut detections, "tool call", 0)
        .expect("a value at the depth filters");
    let nested = GuardrailValue::Object(BTreeMap::from([(
        "a".to_owned(),
        GuardrailValue::Object(BTreeMap::from([("b".to_owned(), text("plain"))])),
    )]));
    let error =
        filter_argument_value(&filter, &nested, &mut detections, "tool call", 0).unwrap_err();
    assert!(matches!(
        error,
        ArgumentFilterError::Depth {
            context_label: "tool call"
        }
    ));
}
