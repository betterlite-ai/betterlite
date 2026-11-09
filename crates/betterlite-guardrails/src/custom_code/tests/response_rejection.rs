use rstest::rstest;

use crate::custom_code::RESPONSE_REJECTION_GUARDRAIL_CODE;
use crate::custom_code_execution::sandbox::DEFAULT_FILENAME;
use crate::custom_code_execution::sandbox::compile_sandboxed;

#[test]
fn the_program_is_a_sandbox_compilable_sync_guardrail() {
    let program = compile_sandboxed(RESPONSE_REJECTION_GUARDRAIL_CODE, DEFAULT_FILENAME)
        .expect("the response rejection program must compile");
    assert_eq!(program.is_async("apply_guardrail"), Some(false));
}

#[rstest]
#[case::thats_not_something("that's not something i can help with")]
#[case::that_is_not_something("that is not something i can help with")]
#[case::cant_help("i can't help with that")]
#[case::cannot_help("i cannot help with that")]
#[case::not_able("i'm not able to help")]
#[case::am_not_able("i am not able to help")]
#[case::unable("i'm unable to help")]
#[case::cannot_assist("i cannot assist")]
#[case::cant_assist("i can't assist")]
#[case::not_allowed("i'm not allowed to")]
#[case::not_permitted("i'm not permitted to")]
#[case::wont_be_able("i won't be able to help")]
#[case::sorry_cant("i'm sorry, i can't")]
#[case::sorry_cannot("i'm sorry, i cannot")]
#[case::as_an_ai_cant("as an ai, i can't")]
#[case::as_an_ai_cannot("as an ai, i cannot")]
fn the_program_lists_litellms_rejection_phrases(#[case] phrase: &str) {
    assert!(RESPONSE_REJECTION_GUARDRAIL_CODE.contains(phrase));
}

#[test]
fn the_program_lowercases_the_text_and_matches_phrases_inside_it() {
    assert!(RESPONSE_REJECTION_GUARDRAIL_CODE.contains("text_lower = lower(text)"));
    assert!(RESPONSE_REJECTION_GUARDRAIL_CODE.contains("if contains(text_lower, phrase):"));
}

#[test]
fn the_program_only_blocks_responses_and_reports_the_matched_phrase() {
    assert!(RESPONSE_REJECTION_GUARDRAIL_CODE.contains("if input_type != \"response\":"));
    assert!(RESPONSE_REJECTION_GUARDRAIL_CODE.contains("rejection_phrases"));
    assert!(RESPONSE_REJECTION_GUARDRAIL_CODE.contains("return block("));
    assert!(RESPONSE_REJECTION_GUARDRAIL_CODE.contains("\"matched_phrase\": phrase"));
    assert!(RESPONSE_REJECTION_GUARDRAIL_CODE.contains("\"input_type\": \"response\""));
}
