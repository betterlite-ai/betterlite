#![cfg(test)]

use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::mcp_guardrail_translation::error::{ContentBlocked, TranslationError};
use crate::mcp_guardrail_translation::process_output::process_output_response;
use crate::mcp_guardrail_translation::tests::{Reply, ScriptedGuardrail, texts, tool_result};

#[tokio::test]
async fn text_blocks_structured_leaves_and_labels_go_out_in_one_call() {
    let mut result = tool_result(
        r#"{"content":[{"type":"text","text":"first"},{"type":"image","data":"AAAA","mimeType":"image/png"},{"type":"text","text":"second"}],
            "structuredContent":{"name":"Ann","age":41}}"#,
    );
    let guardrail = ScriptedGuardrail::new(Reply::Nothing);
    process_output_response(&mut result, &guardrail, None, None)
        .await
        .unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    let (inputs, input_type) = &calls[0];
    assert_eq!(*input_type, GuardrailInputType::Response);
    assert!(inputs.tools.is_empty());
    assert_eq!(
        inputs.texts,
        texts(&["first", "second", "Ann", "age", "name", "41"])
    );
}

#[rstest]
#[case::key(r#"{"secret":"value"}"#)]
#[case::number(r#"{"pin":4242}"#)]
#[tokio::test]
async fn a_masked_label_is_rejected(#[case] structured: &str) {
    let mut result = tool_result(&format!(
        r#"{{"content":[],"structuredContent":{structured}}}"#
    ));
    let secret = if structured.contains("4242") {
        "4242"
    } else {
        "secret"
    };
    let guardrail = ScriptedGuardrail::new(Reply::Mask(secret));
    let outcome = process_output_response(&mut result, &guardrail, None, None).await;
    assert!(matches!(
        outcome,
        Err(TranslationError::Blocked(ContentBlocked::UnrewritableLabel))
    ));
}

#[rstest]
#[case::no_content("{}")]
#[case::only_images(r#"{"content":[{"type":"image","data":"AAAA","mimeType":"image/png"}]}"#)]
#[case::empty_structured_content(r#"{"content":[],"structuredContent":{}}"#)]
#[case::null_structured_content(r#"{"content":[],"structuredContent":null,"isError":false}"#)]
#[tokio::test]
async fn a_result_without_text_skips_the_guardrail(#[case] json: &str) {
    let original = tool_result(json);
    let mut result = original.clone();
    let guardrail = ScriptedGuardrail::new(Reply::Mask("x"));
    process_output_response(&mut result, &guardrail, None, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(result, original);
}

#[tokio::test]
async fn a_guardrail_failure_on_the_result_reaches_the_caller() {
    let mut result = tool_result(r#"{"content":[{"type":"text","text":"x"}]}"#);
    let guardrail = ScriptedGuardrail::new(Reply::Fail);
    let outcome = process_output_response(&mut result, &guardrail, None, None).await;
    assert!(matches!(outcome, Err(TranslationError::Guardrail(_))));
}
