#![cfg(test)]

use rstest::rstest;

use super::prompt_as_messages;
use crate::straiker::v3::fixtures::{NoTokenizer, Tokens, as_json, parsed};

#[rstest]
#[case::string(r#""hello""#, r#"[{"role":"user","content":"hello"}]"#)]
#[case::list_of_strings(
    r#"["a","b"]"#,
    r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#
)]
#[case::token_list_is_decoded(r"[1,2]", r#"[{"role":"user","content":"t1 t2"}]"#)]
#[case::token_lists_are_decoded(
    r"[[1],[2]]",
    r#"[{"role":"user","content":"t1"},{"role":"user","content":"t2"}]"#
)]
#[case::unrenderable_prompt(r"[]", "null")]
#[case::undecodable_tokens(r"[1,500]", "null")]
fn wraps_prompt_as_user_turns(#[case] prompt: &str, #[case] expected: &str) {
    assert_eq!(
        as_json(&prompt_as_messages(&parsed(prompt), &Tokens)),
        parsed(expected)
    );
}

#[test]
fn leaves_token_prompts_alone_without_a_tokenizer() {
    assert_eq!(prompt_as_messages(&parsed("[1,2]"), &NoTokenizer), None);
}
