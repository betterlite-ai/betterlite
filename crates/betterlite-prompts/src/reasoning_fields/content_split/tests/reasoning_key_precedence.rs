use super::*;
use rstest::rstest;

#[rstest]
#[case::reasoning_content_key(
    Some(Content::Text("<think>ignored</think>visible".to_owned())),
    &[("reasoning_content", Json::Text("because".to_owned()))],
    Some("because")
)]
#[case::null_reasoning_content_key_wins_over_tag_parsing(
    Some(Content::Text("<think>ignored</think>visible".to_owned())),
    &[("reasoning_content", Json::Null)],
    None
)]
#[case::reasoning_key(
    None,
    &[("reasoning", Json::Text("why".to_owned()))],
    Some("why")
)]
#[case::both_keys_reasoning_content_wins(
    None,
    &[
        ("reasoning_content", Json::Text("because".to_owned())),
        ("reasoning", Json::Text("why".to_owned())),
    ],
    Some("because")
)]
fn reasoning_fields_behavior_tests_behavior(
    #[case] content: Option<Content>,
    #[case] fields: &[(&str, Json)],
    #[case] expected_reasoning: Option<&str>,
) {
    let provider_specific_fields = Object::from_iter(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone())),
    );
    let input = message(content, provider_specific_fields);
    let expected_content = input.content.clone();

    assert_eq!(
        extract_reasoning_content(&input),
        (expected_reasoning.map(str::to_owned), expected_content)
    );
}
