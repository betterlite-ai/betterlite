use super::BlockCodeExecutionGuardrail;
use crate::block_code_execution::escaped_newlines::normalize_escaped_newlines;
use crate::generic_guardrail_contracts::GuardrailInputType;
use betterlite_config::sections::content_filter_records::BlockCodeAction;

#[test]
fn masks_utf8_and_normalizes_escaped_newlines() {
    assert_eq!(
        normalize_escaped_newlines(r"```py\nprint(1)\n```"),
        "```py\nprint(1)\n```"
    );
    let guardrail = BlockCodeExecutionGuardrail {
        blocked_languages: Some(vec!["python".to_owned()]),
        action: BlockCodeAction::Mask,
        confidence_threshold: 0.5,
        detect_execution_intent: false,
    };
    let mut detections = Vec::new();
    let (output, should_raise) = guardrail.scan_text(
        "\u{e9} \u{1f600}\n```py\nprint(1)\n```\nfin",
        &mut detections,
        GuardrailInputType::Response,
    );
    assert!(!should_raise);
    assert_eq!(output, "\u{e9} \u{1f600}\n[CODE_BLOCK_REDACTED]\nfin");
}
