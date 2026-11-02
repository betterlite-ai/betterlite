#![cfg(test)]

use super::*;
use crate::arguments::json::Object;
use crate::message_content::Content;

fn message(role: &str, text: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Text(text.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn run(kind: MessageKind) -> Run {
    Run {
        kind,
        messages: Vec::new(),
    }
}

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_no_run_precedes_the_first_run();
    assert_a_tool_run_not_preceded_by_system_keeps_only_its_own_messages();
}

// The first run has nothing before it, even when runs[0] happens to carry the system kind itself: index 0 can never look back.
fn assert_no_run_precedes_the_first_run() {
    let runs = vec![run(MessageKind::System), run(MessageKind::Tool)];
    assert!(!previous_run_is_system(&runs, 0));
}

// A tool run whose previous run is a user turn, not a system run, keeps only its own messages instead of also pulling in that user turn's converted form.
fn assert_a_tool_run_not_preceded_by_system_keeps_only_its_own_messages() {
    let messages = vec![
        message("system", "a"),
        message("user", "hi"),
        message("tool", "result"),
    ];
    let expected: Vec<Message> = converted_user_turns(&[message("system", "a")])
        .into_iter()
        .chain([message("user", "hi"), message("tool", "result")])
        .collect();
    assert_eq!(converted_for_unflagged_model(&messages), expected);
}
