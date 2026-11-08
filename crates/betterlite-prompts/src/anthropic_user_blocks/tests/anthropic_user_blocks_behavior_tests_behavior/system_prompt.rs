use std::iter::repeat_with;

use crate::anthropic_history::tools::MAX_HISTORY_MESSAGES;
use crate::anthropic_user_blocks::system_prompt::{SystemPromptError, get_system_prompt};
use crate::arguments::json::Json;

use super::{messages, object};

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_system_prompts_are_concatenated_in_order() {
    let input = messages(
        r#"[{"role":"system","content":"first"},{"role":"user","content":"hi"},{"role":"system","content":" second"}]"#,
    );
    let (prompt, remaining) = get_system_prompt(&input).unwrap();
    assert_eq!(prompt, "first second");
    assert_eq!(remaining, messages(r#"[{"role":"user","content":"hi"}]"#));
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_system_messages_are_left_out_of_the_remaining_list()
 {
    let input = messages(
        r#"[{"role":"system","content":"a"},{"role":"assistant","content":"b"},{"role":"system","content":"c"}]"#,
    );
    let (prompt, remaining) = get_system_prompt(&input).unwrap();
    assert_eq!(prompt, "ac");
    assert_eq!(
        remaining,
        messages(r#"[{"role":"assistant","content":"b"}]"#)
    );
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_a_list_without_system_messages_keeps_every_message()
 {
    let input = messages(r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"ok"}]"#);
    let (prompt, remaining) = get_system_prompt(&input).unwrap();
    assert_eq!(prompt, "");
    assert_eq!(remaining, input);
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_a_non_text_system_message_is_refused() {
    let input = messages(r#"[{"role":"system","content":[{"type":"text","text":"a"}]}]"#);
    let error = get_system_prompt(&input).unwrap_err();
    assert!(matches!(
        error,
        SystemPromptError::ContentNotText { index: 0 }
    ));
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_a_non_object_message_is_refused() {
    let input = vec![Json::Text("not a message".to_owned())];
    let error = get_system_prompt(&input).unwrap_err();
    assert!(matches!(
        error,
        SystemPromptError::MessageNotObject { index: 0 }
    ));
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_too_many_messages_are_refused() {
    let input = repeat_with(|| Json::Object(object(r#"{"role":"system","content":"a"}"#)))
        .take(MAX_HISTORY_MESSAGES + 1)
        .collect::<Vec<_>>();
    let error = get_system_prompt(&input).unwrap_err();
    assert!(matches!(
        error,
        SystemPromptError::TooManyMessages {
            limit: MAX_HISTORY_MESSAGES
        }
    ));
}
