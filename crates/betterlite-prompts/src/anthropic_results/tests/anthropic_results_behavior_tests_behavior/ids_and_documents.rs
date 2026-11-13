use crate::anthropic_results::document_uri::is_anthropic_document_data_uri;
use crate::anthropic_results::ids::{replace_invalid_id_chars, sanitize_anthropic_tool_use_id};

fn is_invalid_bedrock_char(candidate: char) -> bool {
    !(candidate.is_ascii_alphanumeric() || matches!(candidate, '_' | '.' | ':' | '-'))
}

#[rstest::rstest]
#[case::dot_and_slash("call.123/abc", "call_123_abc")]
#[case::valid_characters("abc_DEF-9", "abc_DEF-9")]
#[case::empty_id("", "tool_use_id")]
#[case::non_ascii("\u{fc}mlaut", "_mlaut")]
#[case::space("a b", "a_b")]
fn anthropic_results_behavior_tests_behavior_sanitize_anthropic_ids(
    #[case] raw: &str,
    #[case] expected: &str,
) {
    assert_eq!(sanitize_anthropic_tool_use_id(raw), expected);
}

#[test]
fn anthropic_results_behavior_tests_behavior_the_bedrock_alphabet_keeps_dot_and_colon() {
    assert_eq!(
        replace_invalid_id_chars("a.b:c/d", is_invalid_bedrock_char),
        "a.b:c_d"
    );
}

#[test]
fn anthropic_results_behavior_tests_behavior_the_alphabet_decides_the_replacement() {
    assert_eq!(sanitize_anthropic_tool_use_id("a.b:c"), "a_b_c");
    assert_eq!(
        replace_invalid_id_chars("a.b:c", is_invalid_bedrock_char),
        "a.b:c"
    );
}

#[rstest::rstest]
#[case::pdf("data:application/pdf;base64,QQ==", true)]
#[case::plain_text("data:text/plain,hello", true)]
#[case::plain_text_with_parameters("data:text/plain;charset=utf-8;base64,QQ==", true)]
#[case::png("data:image/png;base64,QQ==", false)]
#[case::empty_media_type("data:;base64,QQ==", false)]
#[case::no_prefix("application/pdf;base64,QQ==", false)]
#[case::leading_space(" data:application/pdf;base64,QQ==", false)]
#[case::uppercase_media_type("data:APPLICATION/PDF;base64,QQ==", false)]
#[case::bare_prefix("data:", false)]
fn anthropic_results_behavior_tests_behavior_document_data_uris(
    #[case] url: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_anthropic_document_data_uri(url), expected);
}
