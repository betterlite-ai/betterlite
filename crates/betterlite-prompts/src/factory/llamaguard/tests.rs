#![cfg(test)]

use super::render;
use crate::builtin::Message;
use crate::factory::Output;
use chrono::{TimeZone as _, Utc};
use rstest::rstest;

fn clock() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn messages(json_text: &str) -> Vec<Message> {
    serde_json::from_str(json_text).unwrap()
}

#[rstest]
#[case::llamaguard_single_user_turn(
    r#"[{"role":"user","content":"hello there"}]"#,
    "User: hello there"
)]
#[case::llamaguard_keeps_upstream_space_before_newline(
    r#"[{"role":"user","content":"hello there"}]"#,
    "O1: Violence and Hate. \nShould not\n"
)]
fn prompt_templates_prompt_template_tests_behavior(#[case] wire: &str, #[case] contains: &str) {
    let output = render(&messages(wire), clock()).unwrap();
    let Output::Text(text) = output else {
        panic!("expected a text output");
    };
    assert!(text.contains(contains));
    assert!(text.contains("safety assessment for User"));
}

#[test]
fn two_consecutive_user_turns_embed_the_alternation_error_instead_of_failing() {
    let input = messages(r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#);
    let output = render(&input, clock()).unwrap();
    let Output::Text(text) = output else {
        panic!("expected a text output");
    };
    assert!(text.contains("Error message - Conversation roles must alternate"));
}
