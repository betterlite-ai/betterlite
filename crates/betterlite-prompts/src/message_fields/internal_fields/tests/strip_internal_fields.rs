use super::*;
use rstest::rstest;

#[rstest]
#[case::no_internal_fields(
    Object::from_iter([("search_results".to_owned(), Json::Text("x".to_owned()))]),
    Object::from_iter([("search_results".to_owned(), Json::Text("x".to_owned()))])
)]
#[case::drops_thinking_blocks(
    Object::from_iter([("thinking_blocks".to_owned(), Json::List(vec![]))]),
    Object::new()
)]
#[case::drops_reasoning_content(
    Object::from_iter([("reasoning_content".to_owned(), Json::Text("because".to_owned()))]),
    Object::new()
)]
#[case::drops_nested_provider_specific_fields(
    Object::from_iter([("provider_specific_fields".to_owned(), Json::Object(Object::new()))]),
    Object::new()
)]
#[case::drops_all_three_keeps_other_keys(
    Object::from_iter([
        ("thinking_blocks".to_owned(), Json::List(vec![])),
        ("reasoning_content".to_owned(), Json::Text("because".to_owned())),
        ("provider_specific_fields".to_owned(), Json::Object(Object::new())),
        ("search_results".to_owned(), Json::Text("x".to_owned())),
    ]),
    Object::from_iter([("search_results".to_owned(), Json::Text("x".to_owned()))])
)]
fn message_fields_behavior_tests_behavior(
    #[case] provider_specific_fields: Object,
    #[case] expected: Object,
) {
    let original = message(provider_specific_fields.clone());
    let stripped = strip_internal_message_fields(&original);
    assert_eq!(stripped.provider_specific_fields, expected);
    assert_eq!(original.provider_specific_fields, provider_specific_fields);
}
