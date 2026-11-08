#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde_json::Number;

use super::{
    INPUT_REQUIRED_UNSUPPORTED, complete_call_tool_result, error_text_result, to_call_tool_result,
    to_gateway_tool,
};
use crate::client::tools::result::{
    CallToolResult, ContentBlock, InputRequiredResult, TextContent, ToolCallResult,
};
use crate::tools::schemas::McpTool;
use crate::wire_compat::tool_outcome::{
    JsonResult, TextResult, ToolFailure, ToolOutcome, WireCompat,
};

fn text_block(text: &str) -> ContentBlock {
    ContentBlock::Text(TextContent {
        text: text.to_owned(),
        extra: Object::new(),
    })
}

fn complete_result(content: Vec<ContentBlock>, structured: Option<Json>) -> CallToolResult {
    CallToolResult {
        content,
        is_error: false,
        structured_content: structured,
        meta: None,
    }
}

fn input_required_result() -> InputRequiredResult {
    InputRequiredResult {
        input_requests: None,
        request_state: Some("state-1".to_owned()),
        result_type: "input_required".to_owned(),
        meta: None,
    }
}

#[rstest]
#[case::legacy(WireCompat::Legacy)]
#[case::modern(WireCompat::Modern)]
fn a_text_outcome_becomes_one_text_block(#[case] compat: WireCompat) {
    let converted = to_call_tool_result(
        ToolOutcome::Text(TextResult {
            text: "hello".to_owned(),
        }),
        compat,
    );
    let ToolCallResult::Complete(result) = converted else {
        panic!("a text outcome converts to a complete result");
    };
    assert_eq!(result.content, [text_block("hello")]);
    assert!(!result.is_error);
    assert!(result.structured_content.is_none());
}

#[rstest]
#[case::legacy_object(WireCompat::Legacy, r#"{"a": 1}"#, true)]
#[case::legacy_scalar(WireCompat::Legacy, "42", false)]
#[case::legacy_array(WireCompat::Legacy, "[1, 2]", false)]
#[case::modern_object(WireCompat::Modern, r#"{"a": 1}"#, true)]
#[case::modern_scalar(WireCompat::Modern, "42", true)]
#[case::modern_array(WireCompat::Modern, "[1, 2]", true)]
fn a_json_outcome_keeps_structured_content_when_the_revision_allows_it(
    #[case] compat: WireCompat,
    #[case] body: &str,
    #[case] keeps: bool,
) {
    let value = serde_json::from_str::<Json>(body).unwrap();
    let converted = to_call_tool_result(
        ToolOutcome::Json(JsonResult {
            value: value.clone(),
            original_text: body.to_owned(),
        }),
        compat,
    );
    let ToolCallResult::Complete(result) = converted else {
        panic!("a json outcome converts to a complete result");
    };
    assert_eq!(result.content, [text_block(body)]);
    assert!(!result.is_error);
    assert_eq!(result.structured_content, keeps.then_some(value));
}

#[rstest]
fn a_legacy_result_downgrades_scalar_structured_content_to_text() {
    let original = complete_result(
        vec![text_block("raw")],
        Some(Json::Number(Number::from(7_i32))),
    );
    let ToolCallResult::Complete(converted) =
        to_call_tool_result(ToolOutcome::Complete(original), WireCompat::Legacy)
    else {
        panic!("an existing result converts to a complete result");
    };
    assert!(converted.structured_content.is_none());
    assert_eq!(converted.content, [text_block("raw"), text_block("7")]);
    assert!(!converted.is_error);
}

#[rstest]
#[case::modern_scalar(WireCompat::Modern, Some(Json::Number(Number::from(7_i32))))]
#[case::legacy_object(WireCompat::Legacy, Some(Json::Object(Object::new())))]
#[case::legacy_none(WireCompat::Legacy, None)]
fn a_result_the_revision_can_carry_is_returned_unchanged(
    #[case] compat: WireCompat,
    #[case] structured: Option<Json>,
) {
    let original = complete_result(vec![text_block("raw")], structured);
    let ToolCallResult::Complete(converted) =
        to_call_tool_result(ToolOutcome::Complete(original.clone()), compat)
    else {
        panic!("an existing result converts to a complete result");
    };
    assert_eq!(converted, original);
}

#[rstest]
fn a_legacy_revision_reports_an_input_required_result_as_an_error() {
    let converted = to_call_tool_result(
        ToolOutcome::InputRequired(input_required_result()),
        WireCompat::Legacy,
    );
    let ToolCallResult::Complete(result) = converted else {
        panic!("a legacy revision cannot carry an interim result");
    };
    assert!(result.is_error);
    assert_eq!(result.content, [text_block(INPUT_REQUIRED_UNSUPPORTED)]);
    assert!(result.structured_content.is_none());
}

#[rstest]
fn a_modern_revision_carries_an_input_required_result_unchanged() {
    let original = input_required_result();
    let converted = to_call_tool_result(
        ToolOutcome::InputRequired(original.clone()),
        WireCompat::Modern,
    );
    assert_eq!(converted, ToolCallResult::InputRequired(original));
}

#[rstest]
fn a_complete_only_caller_reports_a_modern_input_required_result_as_an_error() {
    let converted = complete_call_tool_result(
        ToolOutcome::InputRequired(input_required_result()),
        WireCompat::Modern,
    );
    assert!(converted.is_error);
    assert_eq!(converted.content, [text_block(INPUT_REQUIRED_UNSUPPORTED)]);
}

#[rstest]
fn a_complete_only_caller_returns_the_text_result() {
    let converted = complete_call_tool_result(
        ToolOutcome::Text(TextResult {
            text: "hello".to_owned(),
        }),
        WireCompat::Legacy,
    );
    assert_eq!(converted.content, [text_block("hello")]);
    assert!(!converted.is_error);
}

#[rstest]
fn an_error_outcome_becomes_a_text_block_naming_the_exception() {
    let failure = ToolFailure {
        error_type: "ValueError".to_owned(),
        message: "bad input".to_owned(),
    };
    let converted = to_call_tool_result(ToolOutcome::Failure(failure.clone()), WireCompat::Modern);
    let ToolCallResult::Complete(result) = converted else {
        panic!("an error outcome converts to a complete result");
    };
    assert!(result.is_error);
    assert_eq!(result.content, [text_block("ValueError: bad input")]);
    assert_eq!(result, error_text_result(&failure));
}

#[rstest]
fn a_gateway_tool_keeps_every_field_and_takes_the_gateway_name() {
    let tool = McpTool {
        name: "original".to_owned(),
        description: Some("does things".to_owned()),
        input_schema: Some(Object::from([(
            "type".to_owned(),
            Json::Text("object".to_owned()),
        )])),
        extra: Object::from([("title".to_owned(), Json::Text("Display".to_owned()))]),
    };
    let renamed = to_gateway_tool(&tool, "gateway");
    assert_eq!(renamed.name, "gateway");
    assert_eq!(renamed.description, tool.description);
    assert_eq!(renamed.input_schema, tool.input_schema);
    assert_eq!(renamed.extra, tool.extra);
}

#[rstest]
fn a_gateway_tool_leaves_out_a_missing_description_and_schema() {
    let tool = McpTool {
        name: "original".to_owned(),
        description: None,
        input_schema: None,
        extra: Object::new(),
    };
    let wire = serde_json::to_string(&to_gateway_tool(&tool, "gateway")).unwrap();
    assert_eq!(wire, r#"{"name":"gateway"}"#);
}

#[rstest]
fn a_gateway_tool_drops_null_optionals_and_keeps_the_rest() {
    let tool = McpTool {
        name: "original".to_owned(),
        description: Some("does things".to_owned()),
        input_schema: None,
        extra: Object::from([
            ("title".to_owned(), Json::Null),
            ("outputSchema".to_owned(), Json::Null),
            ("icons".to_owned(), Json::List(Vec::new())),
        ]),
    };
    let wire = serde_json::to_string(&to_gateway_tool(&tool, "gateway")).unwrap();
    assert_eq!(
        wire,
        r#"{"name":"gateway","description":"does things","icons":[]}"#
    );
}
