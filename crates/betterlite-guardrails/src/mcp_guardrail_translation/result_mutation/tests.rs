#![cfg(test)]

use crate::mcp_guardrail_translation::process_output::process_output_response;
use crate::mcp_guardrail_translation::tests::{Reply, ScriptedGuardrail, to_json, tool_result};

#[tokio::test]
async fn the_result_is_masked_in_place_and_keeps_everything_else() {
    let mut result = tool_result(
        r#"{"content":[{"type":"text","text":"key secret-1"},{"type":"image","data":"secret","mimeType":"image/png"},{"type":"text","text":"clean"}],
            "structuredContent":{"note":"secret-2","items":["secret-3","ok"]},"isError":false}"#,
    );
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    process_output_response(&mut result, &guardrail, None, None)
        .await
        .unwrap();
    assert_eq!(
        to_json(&result),
        r#"{"content":[{"text":"key [MASKED]-1","type":"text"},{"data":"secret","mimeType":"image/png","type":"image"},{"text":"clean","type":"text"}],"structuredContent":{"items":["[MASKED]-3","ok"],"note":"[MASKED]-2"},"isError":false}"#
    );
}

#[tokio::test]
async fn snake_case_structured_content_is_scanned_and_masked() {
    let mut result = tool_result(r#"{"content":[],"structured_content":{"note":"secret"}}"#);
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    process_output_response(&mut result, &guardrail, None, None)
        .await
        .unwrap();
    assert_eq!(
        to_json(&result.structured_content),
        r#"{"note":"[MASKED]"}"#
    );
}

#[tokio::test]
async fn text_content_is_written_before_a_label_match_blocks() {
    let mut result = tool_result(
        r#"{"content":[{"type":"text","text":"secret"}],"structuredContent":{"secret":"x"}}"#,
    );
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    let outcome = process_output_response(&mut result, &guardrail, None, None).await;
    assert!(outcome.is_err());
    assert_eq!(result.content[0].text.as_deref(), Some("[MASKED]"));
    assert_eq!(to_json(&result.structured_content), r#"{"secret":"x"}"#);
}
