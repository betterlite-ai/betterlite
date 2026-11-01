use super::*;
use rstest::rstest;

#[rstest]
#[case::empty_messages(vec![], None)]
#[case::no_trailing_user_message(
    vec![text_message("user", "hi"), text_message("assistant", "hello")],
    None
)]
#[case::single_trailing_user_message(
    vec![text_message("assistant", "hello"), text_message("user", "weather?")],
    Some("weather?".to_owned())
)]
#[case::joins_a_consecutive_trailing_run(
    vec![
        text_message("assistant", "hello"),
        text_message("user", "first"),
        text_message("user", "second"),
    ],
    Some("first\nsecond".to_owned())
)]
fn message_fields_behavior_tests_behavior(
    #[case] messages: Vec<Message>,
    #[case] expected: Option<String>,
) {
    assert_eq!(get_last_user_message(&messages), expected);
}
