use rstest::rstest;

use crate::bedrock_tool_results::ids::sanitize_bedrock_tool_use_id;

#[rstest]
#[case::plain_id("call_abc123")]
#[case::dots_and_colons("a.b:c-1_2")]
#[case::already_the_fallback("tool_use_id")]
fn bedrock_tool_results_behavior_tests_behavior_a_valid_id_is_kept(#[case] id: &str) {
    assert_eq!(sanitize_bedrock_tool_use_id(id), id);
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_an_id_at_the_length_limit_is_kept() {
    let id = "x".repeat(64);
    assert_eq!(sanitize_bedrock_tool_use_id(&id), id);
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_an_id_past_the_limit_is_cut_and_hashed() {
    let id = "x".repeat(65);
    assert_eq!(
        sanitize_bedrock_tool_use_id(&id),
        format!("{}_9537c5fd", "x".repeat(55))
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_a_long_id_with_an_invalid_tail_is_cut_and_hashed() {
    let id = format!("{}/", "a".repeat(63));
    assert_eq!(
        sanitize_bedrock_tool_use_id(&id),
        format!("{}_6483da76", "a".repeat(55))
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_an_invalid_character_is_replaced_and_hashed() {
    assert_eq!(sanitize_bedrock_tool_use_id("a.b:c/d"), "a.b:c_d_d9e328ac");
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_an_empty_id_takes_the_fallback_and_is_hashed() {
    assert_eq!(sanitize_bedrock_tool_use_id(""), "tool_use_id_e3b0c442");
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_a_non_ascii_character_is_replaced() {
    assert_eq!(sanitize_bedrock_tool_use_id("\u{fc}"), "__607474ca");
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_a_rewritten_id_ends_with_the_digest_of_the_original()
 {
    let rewritten = sanitize_bedrock_tool_use_id("a b");
    assert!(rewritten.starts_with("a_b_"), "{rewritten}");
    assert_eq!(rewritten.len(), 12);
    assert_ne!(
        sanitize_bedrock_tool_use_id("a_b"),
        rewritten,
        "an id that needs no rewrite keeps no digest"
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_ids_that_differ_past_the_cut_stay_distinct() {
    let first = format!("{}a", "x".repeat(64));
    let second = format!("{}b", "x".repeat(64));
    assert_ne!(
        sanitize_bedrock_tool_use_id(&first),
        sanitize_bedrock_tool_use_id(&second)
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_ids_that_differ_in_a_replaced_character_stay_distinct()
 {
    assert_ne!(
        sanitize_bedrock_tool_use_id("call/1"),
        sanitize_bedrock_tool_use_id("call.1")
    );
}
