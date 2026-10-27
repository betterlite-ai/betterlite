#![cfg(test)]

use crate::mcp_guardrail_translation::json_value::McpValue;
use rstest::rstest;

use crate::mcp_guardrail_translation::error::{ContentBlocked, NestedPayload, TranslationError};
use crate::mcp_guardrail_translation::nesting_depth::{
    Leaf, LeafPath, MAX_STRUCTURED_CONTENT_SCAN_DEPTH, PathPart, scan_json,
};
use crate::mcp_guardrail_translation::process_input::process_input_messages;
use crate::mcp_guardrail_translation::process_output::process_output_response;
use crate::mcp_guardrail_translation::tests::{
    Reply, ScriptedGuardrail, request, texts, tool_result,
};

fn nested(depth: usize, leaf: &str) -> String {
    format!("{}{leaf}{}", "[".repeat(depth), "]".repeat(depth))
}

fn value(json: &str) -> McpValue {
    serde_json::from_str(json).unwrap()
}

#[rstest]
#[case::string_at_the_limit(MAX_STRUCTURED_CONTENT_SCAN_DEPTH, r#""x""#, true)]
#[case::string_past_the_limit(MAX_STRUCTURED_CONTENT_SCAN_DEPTH + 1, r#""x""#, false)]
#[case::number_past_the_limit(MAX_STRUCTURED_CONTENT_SCAN_DEPTH + 1, "1", false)]
#[case::empty_list_at_the_limit(MAX_STRUCTURED_CONTENT_SCAN_DEPTH, "[]", true)]
fn scans_up_to_the_depth_limit(#[case] depth: usize, #[case] leaf: &str, #[case] scans: bool) {
    let outcome = scan_json(&value(&nested(depth, leaf)), NestedPayload::ToolArguments);
    assert_eq!(
        outcome.is_ok(),
        scans,
        "{:?}",
        outcome.map(|found| found.leaves.len())
    );
}

#[test]
fn leaves_come_with_their_paths_and_labels_with_keys_first() {
    let found = scan_json(
        &value(r#"{"b":[1,{"c":"deep"}],"a":"top","d":2.5,"e":true,"f":null}"#),
        NestedPayload::StructuredContent,
    )
    .unwrap();
    assert_eq!(
        found.leaves,
        vec![
            Leaf {
                path: LeafPath(vec![PathPart::Key("a".to_owned())]),
                text: "top".to_owned(),
            },
            Leaf {
                path: LeafPath(vec![
                    PathPart::Key("b".to_owned()),
                    PathPart::Index(1),
                    PathPart::Key("c".to_owned()),
                ]),
                text: "deep".to_owned(),
            },
        ]
    );
    assert_eq!(
        found.labels,
        texts(&["a", "b", "d", "e", "f", "1", "c", "2.5"])
    );
}

#[tokio::test]
async fn arguments_past_the_limit_fail_closed_before_the_guardrail_runs() {
    let mut call = request(&format!(
        r#"{{"mcp_tool_name":"t","mcp_arguments":{}}}"#,
        nested(MAX_STRUCTURED_CONTENT_SCAN_DEPTH + 1, r#""x""#)
    ));
    let guardrail = ScriptedGuardrail::new(Reply::Mask("x"));
    let outcome = process_input_messages(&mut call, &guardrail).await;
    assert!(matches!(
        outcome,
        Err(TranslationError::Blocked(ContentBlocked::TooDeep {
            payload: NestedPayload::ToolArguments
        }))
    ));
    assert!(guardrail.calls().is_empty());
    assert_eq!(ContentBlocked::STATUS.as_u16(), 400);
}

#[tokio::test]
async fn a_schema_past_the_limit_fails_closed() {
    let mut call = request(&format!(
        r#"{{"mcp_tool_name":"t","mcp_input_schema":{{"description":{}}}}}"#,
        nested(MAX_STRUCTURED_CONTENT_SCAN_DEPTH, r#""x""#)
    ));
    let guardrail = ScriptedGuardrail::new(Reply::Nothing);
    let outcome = process_input_messages(&mut call, &guardrail).await;
    assert!(matches!(
        outcome,
        Err(TranslationError::Blocked(ContentBlocked::TooDeep {
            payload: NestedPayload::InputSchema
        }))
    ));
}

#[tokio::test]
async fn structured_content_past_the_limit_fails_closed() {
    let mut result = tool_result(&format!(
        r#"{{"content":[],"structuredContent":{}}}"#,
        nested(MAX_STRUCTURED_CONTENT_SCAN_DEPTH + 1, "0")
    ));
    let guardrail = ScriptedGuardrail::new(Reply::Nothing);
    let outcome = process_output_response(&mut result, &guardrail).await;
    assert!(matches!(
        outcome,
        Err(TranslationError::Blocked(ContentBlocked::TooDeep {
            payload: NestedPayload::StructuredContent
        }))
    ));
    assert!(guardrail.calls().is_empty());
}
