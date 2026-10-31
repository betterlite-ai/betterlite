use super::super::*;
use rstest::rstest;

#[rstest]
#[case::string(Json::Text("prompt".to_owned()), 1)]
#[case::list(
    Json::List(vec![Json::Text("one".to_owned()), Json::Text("two".to_owned())]),
    2
)]
fn message_content_behavior_tests_behavior(#[case] prompt: Json, #[case] count: usize) {
    assert_eq!(to_messages(&prompt).expect("prompt").len(), count);
}
