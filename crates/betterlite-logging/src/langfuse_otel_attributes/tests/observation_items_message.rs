#![cfg(test)]

use super::support::{attr, observation_attributes};

#[test]
fn writes_the_message_role_and_first_content_text() {
    assert_eq!(
        observation_attributes(
            r#"{"output":[{"type":"message","role":"assistant","content":[{"text":"answer"},{"text":"second"}]}]}"#
        ),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"content":"answer","role":"assistant"}]"#
        )]
    );
}

#[test]
fn defaults_the_role_and_the_content() {
    assert_eq!(
        observation_attributes(r#"{"output":[{"type":"message"}]}"#),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"content":"","role":"assistant"}]"#
        )]
    );
}

#[test]
fn output_items_replace_the_choice_output() {
    assert_eq!(
        observation_attributes(
            r#"{"choices":[{"message":{"role":"assistant","content":"first"}}],"output":[{"type":"message","role":"assistant","content":[{"text":"second"}]}]}"#
        ),
        vec![
            attr(
                "langfuse.observation.output",
                r#"{"content":"first","role":"assistant"}"#
            ),
            attr(
                "langfuse.observation.output",
                r#"[{"content":"second","role":"assistant"}]"#
            ),
        ]
    );
}
