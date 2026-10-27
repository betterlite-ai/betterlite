#![cfg(test)]

use rstest::rstest;

use crate::mcp_guardrail_translation::error::{ContentBlocked, ScanTarget, TranslationError};
use crate::mcp_guardrail_translation::process_input::process_input_messages;
use crate::mcp_guardrail_translation::process_output::process_output_response;
use crate::mcp_guardrail_translation::tests::{
    Reply, ScriptedGuardrail, request, texts, tool_result,
};
use crate::mcp_guardrail_translation::text_count_mismatch::masked_texts;

#[rstest]
#[case::fewer(&["a"])]
#[case::more(&["a", "b", "c"])]
#[case::none_at_all(&[])]
fn a_wrong_count_fails_closed(#[case] returned: &[&str]) {
    assert_eq!(
        masked_texts(Some(texts(returned)), 2, ScanTarget::ToolCall),
        Err(ContentBlocked::TextCount {
            returned: returned.len(),
            scanned: 2,
            target: ScanTarget::ToolCall,
        })
    );
}

#[test]
fn the_right_count_and_no_texts_pass() {
    assert_eq!(
        masked_texts(Some(texts(&["a", "b"])), 2, ScanTarget::ToolResult),
        Ok(Some(texts(&["a", "b"])))
    );
    assert_eq!(masked_texts(None, 2, ScanTarget::ToolResult), Ok(None));
}

#[tokio::test]
async fn a_short_reply_leaves_the_tool_call_untouched() {
    let original = request(r#"{"mcp_tool_name":"t","mcp_arguments":{"a":"one","b":"two"}}"#);
    let mut call = original.clone();
    let guardrail = ScriptedGuardrail::new(Reply::Texts(texts(&["[MASKED]"])));
    let outcome = process_input_messages(&mut call, &guardrail).await;
    assert!(matches!(
        outcome,
        Err(TranslationError::Blocked(ContentBlocked::TextCount {
            returned: 1,
            scanned: 2,
            target: ScanTarget::ToolCall,
        }))
    ));
    assert_eq!(call, original);
}

#[tokio::test]
async fn a_long_reply_leaves_the_tool_result_untouched() {
    let original = tool_result(r#"{"content":[{"type":"text","text":"one"}]}"#);
    let mut result = original.clone();
    let guardrail = ScriptedGuardrail::new(Reply::Texts(texts(&["x", "y"])));
    let outcome = process_output_response(&mut result, &guardrail).await;
    assert!(matches!(
        outcome,
        Err(TranslationError::Blocked(ContentBlocked::TextCount {
            returned: 2,
            scanned: 1,
            target: ScanTarget::ToolResult,
        }))
    ));
    assert_eq!(result, original);
}
