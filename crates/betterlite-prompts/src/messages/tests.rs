#![cfg(test)]

use super::{
    detect_first_expected_role, get_completion_messages, insert_assistant_continuation,
    insert_user_continue_message,
};
use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, Message};
use rstest::rstest;
use std::iter::repeat_n;

enum Expected {
    Role(Option<&'static str>),
    Messages(Vec<Message>, bool),
    UserMessages(Vec<Message>, bool),
    AssistantMessages(Vec<Message>, bool),
    CustomMessages(Vec<Message>),
    InputLimit,
    OutputLimit,
}

fn message(role: &str, content: Option<&str>, fields: Object) -> Message {
    Message {
        role: role.to_owned(),
        content: content.map(|text| Content::Text(text.to_owned())),
        name: None,
        provider_specific_fields: fields,
    }
}

fn plain(role: &str, content: Option<&str>) -> Message {
    message(role, content, Object::new())
}

fn tool_call_assistant() -> Message {
    let mut fields = Object::new();
    fields.insert(
        "tool_calls".to_owned(),
        Json::List(vec![Json::Object(Object::new())]),
    );
    message("assistant", None, fields)
}

fn empty_tool_call_assistant() -> Message {
    let mut fields = Object::new();
    fields.insert("tool_calls".to_owned(), Json::List(Vec::new()));
    message("assistant", None, fields)
}

fn signed_assistant() -> Message {
    let mut fields = Object::new();
    fields.insert("signature".to_owned(), Json::Text("keep".to_owned()));
    message("assistant", Some("x"), fields)
}

#[rstest]
#[case::empty_first_role(vec![], Expected::Role(Some("user")))]
#[case::system_then_user(vec![plain("system", None), plain("user", None)], Expected::Role(Some("assistant")))]
#[case::assistant_first(vec![plain("assistant", None)], Expected::Role(Some("user")))]
#[case::only_system(vec![plain("system", None)], Expected::Role(None))]
#[case::insert_before_and_after_assistant(vec![plain("assistant", Some("a"))], Expected::Messages(vec![plain("user", Some("Please continue.")), plain("assistant", Some("a")), plain("user", Some("Please continue."))], true))]
#[case::disabled_returns_copy(vec![plain("assistant", None)], Expected::Messages(vec![plain("assistant", None)], false))]
#[case::empty_conversation(vec![], Expected::Messages(vec![], true))]
#[case::tool_call_assistant_does_not_count(vec![plain("user", None), tool_call_assistant(), plain("tool", None), plain("user", None)], Expected::Messages(vec![plain("user", None), tool_call_assistant(), plain("tool", None), plain("assistant", Some("Please continue.")), plain("user", None)], true))]
#[case::preserves_fields(vec![signed_assistant()], Expected::Messages(vec![plain("user", Some("Please continue.")), signed_assistant(), plain("user", Some("Please continue."))], true))]
#[case::adjacent_counted_assistants(vec![plain("assistant", Some("first")), plain("assistant", Some("second"))], Expected::UserMessages(vec![plain("user", Some("Please continue.")), plain("assistant", Some("first")), plain("user", Some("Please continue.")), plain("assistant", Some("second"))], false))]
#[case::tool_call_assistant_before_counted_assistant(vec![tool_call_assistant(), plain("assistant", Some("counted"))], Expected::UserMessages(vec![plain("user", Some("Please continue.")), tool_call_assistant(), plain("user", Some("Please continue.")), plain("assistant", Some("counted"))], false))]
#[case::counted_assistant_before_tool_then_assistant(vec![plain("assistant", Some("first")), plain("tool", None), plain("assistant", Some("second"))], Expected::UserMessages(vec![plain("user", Some("Please continue.")), plain("assistant", Some("first")), plain("tool", None), plain("user", Some("Please continue.")), plain("assistant", Some("second"))], false))]
#[case::tool_call_assistant_is_not_inserted_after_counted_assistant(vec![plain("assistant", Some("first")), tool_call_assistant()], Expected::UserMessages(vec![plain("user", Some("Please continue.")), plain("assistant", Some("first")), tool_call_assistant()], false))]
#[case::empty_tool_call_list_counts_as_assistant(vec![empty_tool_call_assistant(), plain("assistant", Some("second"))], Expected::UserMessages(vec![plain("user", Some("Please continue.")), empty_tool_call_assistant(), plain("user", Some("Please continue.")), plain("assistant", Some("second"))], false))]
#[case::empty_tool_call_assistant_counts_across_tool_messages(vec![plain("assistant", Some("first")), plain("tool", None), empty_tool_call_assistant(), plain("tool", None), plain("assistant", Some("last"))], Expected::UserMessages(vec![plain("user", Some("Please continue.")), plain("assistant", Some("first")), plain("tool", None), plain("user", Some("Please continue.")), empty_tool_call_assistant(), plain("tool", None), plain("user", Some("Please continue.")), plain("assistant", Some("last"))], false))]
#[case::assistant_continuation_for_consecutive_users(vec![plain("user", Some("first")), plain("user", Some("second"))], Expected::AssistantMessages(vec![plain("user", Some("first")), plain("assistant", Some("Please continue.")), plain("user", Some("second"))], true))]
#[case::assistant_continuation_disabled(vec![plain("user", Some("first")), plain("user", Some("second"))], Expected::AssistantMessages(vec![plain("user", Some("first")), plain("user", Some("second"))], false))]
#[case::custom_continuations(vec![plain("assistant", Some("start")), plain("user", Some("first")), plain("user", Some("second"))], Expected::CustomMessages(vec![plain("user", Some("go on")), plain("assistant", Some("start")), plain("user", Some("first")), plain("assistant", Some("go on")), plain("user", Some("second"))]))]
#[case::input_limit_returns_named_error(vec![], Expected::InputLimit)]
#[case::output_limit_returns_named_error(vec![], Expected::OutputLimit)]
fn message_normalization_message_tests_behavior(
    #[case] input: Vec<Message>,
    #[case] expected: Expected,
) {
    match expected {
        Expected::Role(expected_role) => {
            assert_eq!(detect_first_expected_role(&input), expected_role);
        }
        Expected::Messages(expected_messages, ensure) => {
            let original = input.clone();
            let actual = get_completion_messages(&input, None, None, ensure).unwrap();
            assert_eq!(actual, expected_messages);
            assert_eq!(input, original);
        }
        Expected::UserMessages(expected_messages, ensure) => {
            let original = input.clone();
            let actual = insert_user_continue_message(&input, None, ensure).unwrap();
            assert_eq!(actual, expected_messages);
            assert_eq!(input, original);
        }
        Expected::AssistantMessages(expected_messages, ensure) => {
            let original = input.clone();
            let actual = insert_assistant_continuation(&input, None, ensure).unwrap();
            assert_eq!(actual, expected_messages);
            assert_eq!(input, original);
        }
        Expected::CustomMessages(expected_messages) => {
            let user = plain("user", Some("go on"));
            let assistant = plain("assistant", Some("go on"));
            let actual =
                get_completion_messages(&input, Some(&assistant), Some(&user), true).unwrap();
            assert_eq!(actual, expected_messages);
        }
        Expected::InputLimit => {
            let oversized = repeat_n(plain("user", None), super::MAX_COMPLETION_MESSAGES + 1)
                .collect::<Vec<_>>();
            assert_eq!(
                get_completion_messages(&oversized, None, None, true).unwrap_err(),
                super::MessageNormalizationError::InputTooLong {
                    limit: super::MAX_COMPLETION_MESSAGES
                }
            );
        }
        Expected::OutputLimit => {
            let oversized =
                repeat_n(plain("user", None), super::MAX_COMPLETION_MESSAGES).collect::<Vec<_>>();
            assert_eq!(
                get_completion_messages(&oversized, None, None, true).unwrap_err(),
                super::MessageNormalizationError::OutputTooLong {
                    limit: super::MAX_COMPLETION_MESSAGES
                }
            );
        }
    }
}
