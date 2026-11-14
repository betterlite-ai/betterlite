#![cfg(test)]

use super::*;
use rstest::rstest;
use serde_json::{Value, json};

fn close(actual: f64, expected: f64) -> bool {
    expected.mul_add(-1.0, actual).abs() < 1e-9
}

fn scores(query: &str, messages: &[Value]) -> Vec<f64> {
    bm25_score_messages(
        query,
        messages,
        DEFAULT_TERM_SATURATION,
        DEFAULT_LENGTH_NORMALIZATION,
    )
}

#[test]
fn tokenizes_on_word_boundaries_into_lowercase_tokens() {
    assert_eq!(
        tokenize("Hello, WORLD_2 foo-bar 3x"),
        ["hello", "world_2", "foo", "bar", "3x"]
    );
}

#[test]
fn lowercases_unicode_before_tokenizing() {
    assert_eq!(tokenize("İ K"), ["i", "k"]);
}

#[test]
fn extracts_text_blocks_and_string_parts() {
    let content_parts = json!([
        {"type": "text", "text": "def login_handler"},
        {"type": "image_url", "image_url": {"url": "x"}},
        "login handler"
    ]);
    let plain = json!({"role": "user", "content": "def login_handler login handler"});
    let list_content = json!({"role": "user", "content": content_parts});
    let filler = json!({"role": "user", "content": "x"});
    let result = scores("login handler", &[list_content, filler.clone()]);
    assert_eq!(
        result,
        scores("login handler", &[plain, filler]),
        "a block list reads like its joined text"
    );
    assert!(close(result[0], 1.091_570_363_086_528));
}

#[test]
fn unrolls_nested_tool_result_content() {
    let nested = json!({
        "role": "user",
        "content": [{"type": "tool_result", "content": [{"type": "text", "text": "nested"}]}],
    });
    let plain = json!({"role": "user", "content": "nested"});
    assert_eq!(scores("nested", &[nested]), scores("nested", &[plain]));
}

#[test]
fn ranks_messages_that_share_query_terms() {
    let messages = vec![
        json!({"role": "user", "content": "authentication bug fix"}),
        json!({"role": "user", "content": "unrelated content"}),
        json!({"role": "user", "content": "bugs authentication authentication"}),
    ];
    let result = scores("authentication bug", &messages);
    assert_eq!(result.len(), 3);
    assert!(close(result[0], 1.373_569_592_669_786_4));
    assert!(close(result[1], 0.0));
    assert!(close(result[2], 0.645_498_546_603_585_4));
    assert!(result[0] > result[1]);
    assert!(result[2] > result[1]);
}

#[test]
fn scores_follow_the_bm25_formula() {
    let messages = vec![
        json!({"role": "user", "content": "authentication bug fix"}),
        json!({"role": "user", "content": "unrelated content"}),
    ];
    let result = scores("authentication bug", &messages);
    assert!(close(result[0], 1.271_829_689_100_817_2));
    assert!(close(result[1], 0.0));
}

#[test]
fn uses_the_supplied_saturation_and_length_parameters() {
    let messages = vec![
        json!({"content": "authentication bug fix"}),
        json!({"content": "unrelated content"}),
        json!({"content": "bugs authentication authentication"}),
    ];
    let result = bm25_score_messages("authentication bug", &messages, 1.2, 0.5);
    assert!(close(result[0], 1.403_003_226_798_425));
    assert!(close(result[2], 0.631_455_257_612_591_5));
}

#[test]
fn default_parameters_match_litellm() {
    assert_eq!(DEFAULT_TERM_SATURATION.to_bits(), 1.5_f64.to_bits());
    assert_eq!(DEFAULT_LENGTH_NORMALIZATION.to_bits(), 0.75_f64.to_bits());
}

#[test]
fn returns_zero_scores_when_the_query_has_no_terms() {
    let messages = vec![json!({"content": "hello"}), json!({"content": ""})];
    assert_eq!(scores("", &messages), vec![0.0, 0.0]);
    assert_eq!(scores(" ,.! ", &messages), vec![0.0, 0.0]);
}

#[test]
fn returns_no_scores_for_no_messages() {
    assert!(scores("query", &[]).is_empty());
}

#[test]
fn scores_documents_without_tokens_zero() {
    let messages = vec![json!({"content": ""}), json!({"content": "auth"})];
    let result = scores("auth", &messages);
    assert!(close(result[0], 0.0));
    assert!(close(result[1], 0.478_032_538_317_203_66));
}

#[test]
fn returns_zeros_when_every_document_is_empty() {
    let messages = vec![json!({"content": ""}), json!({"content": "  "})];
    assert_eq!(scores("query", &messages), vec![0.0, 0.0]);
}

#[test]
fn exact_matches_outrank_prefix_only_matches() {
    let messages = vec![
        json!({"content": "authentication authentication_a"}),
        json!({"content": "authentication_a authentication_b"}),
    ];
    let result = scores("authentication", &messages);
    assert!(close(result[0], 0.693_147_180_559_945_3));
    assert!(close(result[1], 0.990_210_257_942_779_1));
    assert!(result[1] > result[0]);
}

#[test]
fn sums_term_frequency_across_every_matching_prefix_token() {
    let messages = vec![
        json!({"content": "authentication authentication"}),
        json!({"content": "authentication_a authentication_b"}),
    ];
    let result = scores("authentication", &messages);
    assert!(close(result[0], 0.990_210_257_942_779_1));
    assert!(close(result[1], 0.990_210_257_942_779_1));
}

#[rstest]
#[case::four_character_prefix("auth", "authentication", true)]
#[case::three_character_prefix("aut", "authentication", false)]
#[case::three_character_unrelated("cat", "category", false)]
#[case::four_character_not_a_prefix("cats", "category", false)]
fn prefix_matching_starts_at_four_characters(
    #[case] query: &str,
    #[case] content: &str,
    #[case] matches: bool,
) {
    let result = scores(query, &[json!({"content": content})]);
    if matches {
        assert!(result[0] > 0.0);
    } else {
        assert!(close(result[0], 0.0));
    }
}

#[test]
fn repeated_query_terms_add_their_contributions() {
    let messages = vec![json!({"content": "auth"})];
    let single = scores("auth", &messages);
    let repeated = scores("auth auth", &messages);
    assert!(close(single[0], 0.287_682_072_451_780_85));
    assert!(close(repeated[0], 0.575_364_144_903_561_7));
}

#[test]
fn penalizes_longer_documents_with_equal_term_frequency() {
    let messages = vec![
        json!({"content": "term"}),
        json!({"content": "term filler filler filler filler"}),
    ];
    let result = scores("term", &messages);
    assert!(close(result[0], 0.260_459_366_848_506_57));
    assert!(close(result[1], 0.140_247_351_379_965_08));
    assert!(result[0] > result[1]);
}
