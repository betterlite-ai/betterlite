#![cfg(test)]

use super::{detect_first_expected_role, get_completion_messages};
use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, Message};
use rstest::rstest;

enum Expected {
    Role(Option<&'static str>),
    Messages(Vec<Message>, bool),
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
            let actual = get_completion_messages(&input, None, None, ensure);
            assert_eq!(actual, expected_messages);
            assert_eq!(input, original);
        }
    }
}
