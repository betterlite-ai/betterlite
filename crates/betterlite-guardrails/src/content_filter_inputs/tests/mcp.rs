#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ContentFilterAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use crate::content_filter_inputs::arguments::ArgumentFilter;
use crate::content_filter_inputs::mcp::{MCP_CALL_TYPE, scan_mcp_tool_call_arguments};

use super::{Fixture, base, pattern_names, pre_mcp_call_hook, regex_pattern};

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn request_data(name: &str, arguments: GuardrailValue) -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        ("mcp_tool_name".to_owned(), text(name)),
        ("mcp_arguments".to_owned(), arguments),
    ])
}

fn secret_fixture() -> Fixture {
    Fixture::with_patterns(vec![regex_pattern(
        "secret",
        "secret",
        ContentFilterAction::Mask,
    )])
}

fn secret_arguments() -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([(
        "query".to_owned(),
        text("a secret value"),
    )]))
}

enum ToolName {
    Missing,
    Blank,
    NotText,
}

enum ArgumentsShape {
    Missing,
    Text,
    Empty,
}

#[test]
fn mcp_tool_argument_filtering() {
    let fixture = secret_fixture();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut data = request_data("search", secret_arguments());
    let mut detections = Vec::new();
    scan_mcp_tool_call_arguments(
        &base(Some(pre_mcp_call_hook())),
        &filter,
        Some(MCP_CALL_TYPE),
        &mut data,
        &mut detections,
    )
    .expect("the MCP arguments filter");
    let expected = GuardrailValue::Object(BTreeMap::from([(
        "query".to_owned(),
        text("a [SECRET_REDACTED] value"),
    )]));
    assert_eq!(data.get("mcp_arguments"), Some(&expected));
    assert_eq!(data.get("modified_arguments"), Some(&expected));
    assert_eq!(pattern_names(&detections), ["secret"]);
}

#[test]
fn mcp_call_skips_without_the_pre_mcp_call_hook() {
    let fixture = secret_fixture();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let arguments = secret_arguments();
    let mut data = request_data("search", arguments.clone());
    let mut detections = Vec::new();
    let hook = Some(LitellmParamsMode::One("pre_call".to_owned()));
    scan_mcp_tool_call_arguments(&base(hook), &filter, None, &mut data, &mut detections)
        .expect("the scan skips");
    assert_eq!(data.get("mcp_arguments"), Some(&arguments));
    assert!(!data.contains_key("modified_arguments"));
    assert!(detections.is_empty());
}

#[test]
fn mcp_call_skips_other_call_types() {
    let fixture = secret_fixture();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let arguments = secret_arguments();
    let mut other = request_data("search", arguments.clone());
    let mut detections = Vec::new();
    scan_mcp_tool_call_arguments(
        &base(Some(pre_mcp_call_hook())),
        &filter,
        Some("acompletion"),
        &mut other,
        &mut detections,
    )
    .expect("another call type skips");
    assert_eq!(other.get("mcp_arguments"), Some(&arguments));
    assert!(!other.contains_key("modified_arguments"));
    assert!(detections.is_empty());
    let mut untyped = request_data("search", arguments);
    scan_mcp_tool_call_arguments(
        &base(Some(pre_mcp_call_hook())),
        &filter,
        None,
        &mut untyped,
        &mut detections,
    )
    .expect("a call without a logging object filters");
    assert!(untyped.contains_key("modified_arguments"));
}

#[rstest]
#[case::missing(ToolName::Missing)]
#[case::blank(ToolName::Blank)]
#[case::not_text(ToolName::NotText)]
fn mcp_call_skips_without_a_readable_tool_name(#[case] name: ToolName) {
    let fixture = secret_fixture();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let arguments = secret_arguments();
    let mut data = match name {
        ToolName::Missing => BTreeMap::from([("mcp_arguments".to_owned(), arguments.clone())]),
        ToolName::Blank => request_data("", arguments.clone()),
        ToolName::NotText => {
            let mut built = request_data("search", arguments.clone());
            built.insert("mcp_tool_name".to_owned(), GuardrailValue::SignedInteger(7));
            built
        }
    };
    let mut detections = Vec::new();
    scan_mcp_tool_call_arguments(
        &base(Some(pre_mcp_call_hook())),
        &filter,
        Some(MCP_CALL_TYPE),
        &mut data,
        &mut detections,
    )
    .expect("a call without a tool name skips");
    assert_eq!(data.get("mcp_arguments"), Some(&arguments));
    assert!(!data.contains_key("modified_arguments"));
    assert!(detections.is_empty());
}

#[rstest]
#[case::missing(ArgumentsShape::Missing)]
#[case::text(ArgumentsShape::Text)]
#[case::empty(ArgumentsShape::Empty)]
fn mcp_call_skips_without_readable_arguments(#[case] shape: ArgumentsShape) {
    let fixture = secret_fixture();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let mut data = match shape {
        ArgumentsShape::Missing => BTreeMap::from([("mcp_tool_name".to_owned(), text("search"))]),
        ArgumentsShape::Text => request_data("search", text("a secret value")),
        ArgumentsShape::Empty => request_data("search", GuardrailValue::Object(BTreeMap::new())),
    };
    let mut detections = Vec::new();
    scan_mcp_tool_call_arguments(
        &base(Some(pre_mcp_call_hook())),
        &filter,
        Some(MCP_CALL_TYPE),
        &mut data,
        &mut detections,
    )
    .expect("a call without readable arguments skips");
    assert!(!data.contains_key("modified_arguments"));
    assert!(detections.is_empty());
}

#[test]
fn mcp_call_checks_object_keys() {
    let fixture = secret_fixture();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let arguments = GuardrailValue::Object(BTreeMap::from([("secret".to_owned(), text("plain"))]));
    let mut data = request_data("search", arguments.clone());
    let mut detections = Vec::new();
    let error = scan_mcp_tool_call_arguments(
        &base(Some(pre_mcp_call_hook())),
        &filter,
        Some(MCP_CALL_TYPE),
        &mut data,
        &mut detections,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Content blocked: MCP tool call argument matched a masking rule on a non-rewritable field"
    );
    assert_eq!(data.get("mcp_arguments"), Some(&arguments));
}

#[test]
fn mcp_call_depth_limit_names_the_mcp_label() {
    let fixture = Fixture::empty();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 1,
    };
    let inner = GuardrailValue::Object(BTreeMap::from([("b".to_owned(), text("leaf"))]));
    let arguments = GuardrailValue::Object(BTreeMap::from([("a".to_owned(), inner)]));
    let mut data = request_data("search", arguments);
    let mut detections = Vec::new();
    let error = scan_mcp_tool_call_arguments(
        &base(Some(pre_mcp_call_hook())),
        &filter,
        Some(MCP_CALL_TYPE),
        &mut data,
        &mut detections,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Content blocked: MCP tool call arguments exceed the maximum nesting depth"
    );
}

#[test]
fn mcp_call_keeps_unchanged_arguments_unmodified() {
    let fixture = secret_fixture();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 100,
    };
    let arguments = GuardrailValue::Object(BTreeMap::from([(
        "a".to_owned(),
        GuardrailValue::Boolean(true),
    )]));
    let mut data = request_data("search", arguments.clone());
    let mut detections = Vec::new();
    scan_mcp_tool_call_arguments(
        &base(Some(pre_mcp_call_hook())),
        &filter,
        Some(MCP_CALL_TYPE),
        &mut data,
        &mut detections,
    )
    .expect("unchanged arguments filter");
    assert_eq!(data.get("mcp_arguments"), Some(&arguments));
    assert!(!data.contains_key("modified_arguments"));
    assert!(detections.is_empty());
}
