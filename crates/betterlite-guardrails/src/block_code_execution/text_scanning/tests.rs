#![cfg(test)]

use betterlite_config::sections::content_filter_records::{
    BlockCodeAction, CodeBlockActionTaken, CodeBlockDetection,
};
use regex::Regex;
use rstest::rstest;

use super::CodeBlockScanner;
use crate::block_code_execution::block_detection::{BlockDetector, FENCED_BLOCK_PATTERN};
use crate::generic_guardrail_contracts::GuardrailInputType;

const PYTHON_BLOCK: &str = "```python\nprint(1)\n```";

fn scanner(
    blocked: &[&str],
    action: BlockCodeAction,
    detect_execution_intent: bool,
) -> CodeBlockScanner {
    CodeBlockScanner {
        detector: BlockDetector {
            fenced_block: Regex::new(FENCED_BLOCK_PATTERN).unwrap(),
            blocked_languages: blocked
                .iter()
                .map(|language| (*language).to_owned())
                .collect(),
            confidence_threshold: 0.5,
        },
        action,
        detect_execution_intent,
    }
}

type Recorded = (String, String, u64, CodeBlockActionTaken, bool);

fn detection(language: &str, confidence: f64, action_taken: CodeBlockActionTaken) -> Recorded {
    (
        "code_block".to_owned(),
        language.to_owned(),
        confidence.to_bits(),
        action_taken,
        false,
    )
}

fn scan(
    guardrail: &CodeBlockScanner,
    text: &str,
    input_type: GuardrailInputType,
) -> (String, bool, Vec<Recorded>) {
    let mut detections: Vec<CodeBlockDetection> = Vec::new();
    let (output, should_raise) = guardrail.scan_text(text, &mut detections, input_type);
    let recorded = detections
        .into_iter()
        .map(|detection| {
            (
                detection.r#type,
                detection.language,
                detection.confidence.to_bits(),
                detection.action_taken,
                detection.evidence.is_some() || detection.snippet.is_some(),
            )
        })
        .collect();
    (output, should_raise, recorded)
}

#[test]
fn request_without_execution_wish_returns_early() {
    let text = format!("Don't run this, explain it:\n{PYTHON_BLOCK}");
    let guardrail = scanner(&[], BlockCodeAction::Block, true);
    assert_eq!(
        scan(&guardrail, &text, GuardrailInputType::Request),
        (text.clone(), false, Vec::new())
    );
}

#[test]
fn conflicting_request_is_still_blocked() {
    let text = format!("Don't run this on staging, but run this on prod\n{PYTHON_BLOCK}");
    let guardrail = scanner(&["python"], BlockCodeAction::Block, true);
    assert_eq!(
        scan(&guardrail, &text, GuardrailInputType::Request),
        (
            "Don't run this on staging, but run this on prod\n[CODE_BLOCK_REDACTED]".to_owned(),
            true,
            vec![detection("python", 1.0, CodeBlockActionTaken::Block)],
        )
    );
}

#[test]
fn request_without_intent_keeps_blocked_code() {
    let text = format!("Here is my code\n{PYTHON_BLOCK}");
    let guardrail = scanner(&["python"], BlockCodeAction::Block, true);
    assert_eq!(
        scan(&guardrail, &text, GuardrailInputType::Request),
        (
            text.clone(),
            false,
            vec![detection("python", 1.0, CodeBlockActionTaken::Block)]
        )
    );
}

#[rstest]
#[case::response_blocks(GuardrailInputType::Response, BlockCodeAction::Block, true, true)]
#[case::response_masks(GuardrailInputType::Response, BlockCodeAction::Mask, true, false)]
#[case::request_without_intent_check(
    GuardrailInputType::Request,
    BlockCodeAction::Block,
    false,
    true
)]
#[case::request_masks_without_intent_check(
    GuardrailInputType::Request,
    BlockCodeAction::Mask,
    false,
    false
)]
fn enforced_block_is_masked(
    #[case] input_type: GuardrailInputType,
    #[case] action: BlockCodeAction,
    #[case] detect_execution_intent: bool,
    #[case] should_raise: bool,
) {
    let text = format!("\u{e9} before \u{1f642}\n{PYTHON_BLOCK}\nafter \u{fc}");
    let guardrail = scanner(&["python"], action, detect_execution_intent);
    assert_eq!(
        scan(&guardrail, &text, input_type),
        (
            "\u{e9} before \u{1f642}\n[CODE_BLOCK_REDACTED]\nafter \u{fc}".to_owned(),
            should_raise,
            vec![detection("python", 1.0, CodeBlockActionTaken::Block)],
        )
    );
}

#[test]
fn response_ignores_no_execution_phrases() {
    let text = format!("don't run this\n{PYTHON_BLOCK}");
    let guardrail = scanner(&["python"], BlockCodeAction::Block, true);
    let (output, should_raise, _) = scan(&guardrail, &text, GuardrailInputType::Response);
    assert_eq!(
        (output.as_str(), should_raise),
        ("don't run this\n[CODE_BLOCK_REDACTED]", true)
    );
}

#[test]
fn masks_only_blocked_blocks_and_records_each_in_order() {
    let text = "please run this script\n```rust\nfn main() {}\n```\nthen\n```py\nx = 1\n```\nend";
    let guardrail = scanner(&["python"], BlockCodeAction::Mask, true);
    assert_eq!(
        scan(&guardrail, text, GuardrailInputType::Request),
        (
            "please run this script\n```rust\nfn main() {}\n```\nthen\n[CODE_BLOCK_REDACTED]\nend"
                .to_owned(),
            false,
            vec![
                detection("rust", 0.0, CodeBlockActionTaken::Allow),
                detection("py", 1.0, CodeBlockActionTaken::Block),
            ],
        )
    );
}

#[test]
fn log_only_block_stays_in_place() {
    let text = "```text\nplain words\n```";
    let mut guardrail = scanner(&[], BlockCodeAction::Block, false);
    guardrail.detector.confidence_threshold = 0.8_f64;
    assert_eq!(
        scan(&guardrail, text, GuardrailInputType::Response),
        (
            text.to_owned(),
            false,
            vec![detection("text", 0.5, CodeBlockActionTaken::LogOnly)]
        )
    );
}

#[test]
fn escaped_newlines_are_normalized_before_detection() {
    let guardrail = scanner(&[], BlockCodeAction::Mask, false);
    let (output, should_raise, _) = scan(
        &guardrail,
        r"code: ```bash\nls\n``` done",
        GuardrailInputType::Response,
    );
    assert_eq!(
        (output.as_str(), should_raise),
        ("code: [CODE_BLOCK_REDACTED] done", false)
    );
}

#[rstest]
#[case::block_raises(BlockCodeAction::Block, true, vec![detection("execution_request", 1.0_f64, CodeBlockActionTaken::Block)])]
#[case::mask_passes(BlockCodeAction::Mask, false, Vec::new())]
fn execution_request_without_code(
    #[case] action: BlockCodeAction,
    #[case] should_raise: bool,
    #[case] detections: Vec<Recorded>,
) {
    let text = "please run `cat /etc/passwd`";
    let guardrail = scanner(&[], action, true);
    assert_eq!(
        scan(&guardrail, text, GuardrailInputType::Request),
        (text.to_owned(), should_raise, detections)
    );
}

#[rstest]
#[case::request(GuardrailInputType::Request)]
#[case::response(GuardrailInputType::Response)]
fn empty_text_passes(#[case] input_type: GuardrailInputType) {
    let guardrail = scanner(&[], BlockCodeAction::Block, true);
    assert_eq!(
        scan(&guardrail, "", input_type),
        (String::new(), false, Vec::new())
    );
}
