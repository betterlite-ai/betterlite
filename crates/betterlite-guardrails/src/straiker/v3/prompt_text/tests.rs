#![cfg(test)]

use rstest::rstest;

use super::{DavinciDecoder, PromptTokenDecoder, prompt_texts};
use crate::straiker::v3::fixtures::{NoTokenizer, Tokens, as_json, parsed};

#[rstest]
#[case::string(r#""hello""#, r#"["hello"]"#)]
#[case::empty_string(r#""""#, r#"[""]"#)]
#[case::list_of_strings(r#"["a","","b"]"#, r#"["a","","b"]"#)]
#[case::token_list(r"[1,2,3]", r#"["t1 t2 t3"]"#)]
#[case::list_of_token_lists(r"[[1,2],[3]]", r#"["t1 t2","t3"]"#)]
#[case::empty_list(r"[]", "null")]
#[case::number(r"7", "null")]
#[case::null(r"null", "null")]
#[case::object(r#"{"a":1}"#, "null")]
#[case::mixed_strings_and_tokens(r#"["a",1]"#, "null")]
#[case::token_list_with_float(r"[1,2.5]", "null")]
#[case::token_list_with_bool(r"[1,true]", "null")]
#[case::token_list_with_negative_id(r"[1,-2]", "null")]
#[case::token_list_with_oversized_id(r"[1,4294967296]", "null")]
#[case::undecodable_id(r"[1,500]", "null")]
#[case::one_undecodable_list_fails_all(r"[[1],[500]]", "null")]
#[case::list_with_an_empty_token_list(r"[[1],[]]", "null")]
#[case::list_of_token_lists_with_a_string(r#"[[1],"a"]"#, "null")]
fn renders_prompt(#[case] prompt: &str, #[case] expected: &str) {
    assert_eq!(
        as_json(&prompt_texts(&parsed(prompt), &Tokens)),
        parsed(expected)
    );
}

#[rstest]
#[case::token_list(r"[1,2,3]")]
#[case::list_of_token_lists(r"[[1,2],[3]]")]
fn relays_token_prompts_untouched_without_a_tokenizer(#[case] prompt: &str) {
    assert_eq!(prompt_texts(&parsed(prompt), &NoTokenizer), None);
}

#[rstest]
#[case::string(r#""hello""#)]
#[case::list_of_strings(r#"["a","b"]"#)]
fn needs_no_tokenizer_for_text(#[case] prompt: &str) {
    assert!(prompt_texts(&parsed(prompt), &NoTokenizer).is_some());
}

#[rstest]
#[case::token_list(r"[31373,995]", r#"["hello world"]"#)]
#[case::list_of_token_lists(r"[[31373],[995]]", r#"["hello"," world"]"#)]
#[case::out_of_vocabulary_id(r"[31373,999999999]", "null")]
#[case::one_out_of_vocabulary_list_fails_all(r"[[31373],[999999999]]", "null")]
fn decodes_prompts_with_text_davinci_003(#[case] prompt: &str, #[case] expected: &str) {
    assert_eq!(
        as_json(&prompt_texts(&parsed(prompt), &DavinciDecoder::new())),
        parsed(expected)
    );
}

#[test]
fn davinci_decoder_reads_p50k_base() {
    let decoder = DavinciDecoder::default();
    assert_eq!(
        decoder.decode(&[31373, 995]).as_deref(),
        Some("hello world")
    );
    assert_eq!(decoder.decode(&[999_999_999]), None);
    assert_eq!(decoder.decode(&[]).as_deref(), Some(""));
}
