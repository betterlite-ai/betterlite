#![cfg(test)]

use betterlite_models::prompt_contracts::AllMessageValues;
use indexmap::IndexMap;
use rstest::rstest;

use super::convert_to_messages;

fn message(role: &str, content: &str) -> AllMessageValues {
    let mut fields = IndexMap::new();
    fields.insert("role".to_owned(), AllMessageValues::Text(role.to_owned()));
    fields.insert(
        "content".to_owned(),
        AllMessageValues::Text(content.to_owned()),
    );
    AllMessageValues::Object(fields)
}

#[rstest]
#[case::role_prefixes_split_into_separate_messages(
    "System: setup\nUser: hello",
    vec![message("system", "setup"), message("user", "hello")]
)]
#[case::lowercase_prefixes_are_not_role_markers("system: setup", vec![message("user", "system: setup")])]
#[case::content_with_no_role_prefix_is_a_single_user_message(
    "just some text\nacross two lines",
    vec![message("user", "just some text\nacross two lines")]
)]
#[case::an_empty_message_between_two_role_switches_is_still_added(
    "User:\nAssistant: hi",
    vec![message("user", ""), message("assistant", "hi")]
)]
#[case::trailing_empty_content_is_dropped("User: hi\nAssistant:", vec![message("user", "hi")])]
#[case::empty_input_produces_no_messages("", vec![])]
#[case::whitespace_only_input_produces_no_messages("   \n  ", vec![])]
fn converts_rendered_content_to_messages(
    #[case] rendered_content: &str,
    #[case] expected: Vec<AllMessageValues>,
) {
    assert_eq!(convert_to_messages(rendered_content), expected);
}
