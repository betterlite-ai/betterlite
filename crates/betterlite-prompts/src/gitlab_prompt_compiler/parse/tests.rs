#![cfg(test)]

use rstest::rstest;

use super::parse_prompt_to_messages;

fn messages(prompt_content: &str) -> String {
    serde_json::to_string(&parse_prompt_to_messages(prompt_content)).unwrap()
}

#[rstest]
#[case::three_roles(
    "System: a\nUser: b\nAssistant: c",
    r#"[{"role":"system","content":"a"},{"role":"user","content":"b"},{"role":"assistant","content":"c"}]"#
)]
#[case::role_prefixes_are_case_insensitive(
    "sYsTeM: a\nUSER: b\nAssistant: c",
    r#"[{"role":"system","content":"a"},{"role":"user","content":"b"},{"role":"assistant","content":"c"}]"#
)]
#[case::blank_lines_are_skipped("User: a\n\n   \nb", r#"[{"role":"user","content":"a\nb"}]"#)]
#[case::text_before_a_role_is_dropped("intro\nUser: hi", r#"[{"role":"user","content":"hi"}]"#)]
#[case::text_without_roles_becomes_one_user_message(
    "hello\n\nworld",
    r#"[{"role":"user","content":"hello\n\nworld"}]"#
)]
#[case::surrounding_whitespace_is_trimmed(
    "  User:   hi   \n  there  ",
    r#"[{"role":"user","content":"hi\nthere"}]"#
)]
#[case::a_role_with_an_empty_first_line_takes_the_next_line(
    "User:\nhello",
    r#"[{"role":"user","content":"hello"}]"#
)]
#[case::a_role_alone_keeps_an_empty_message("User:", r#"[{"role":"user","content":""}]"#)]
#[case::a_longer_word_is_not_a_role("Users: hi", r#"[{"role":"user","content":"Users: hi"}]"#)]
#[case::two_messages_with_the_same_role(
    "User: a\nuser: b",
    r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#
)]
#[case::empty_content("", "[]")]
#[case::blank_content("  \n\n ", "[]")]
fn the_template_becomes_role_messages(#[case] prompt_content: &str, #[case] expected: &str) {
    assert_eq!(messages(prompt_content), expected);
}
