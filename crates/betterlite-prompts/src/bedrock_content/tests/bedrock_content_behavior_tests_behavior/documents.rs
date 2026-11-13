use crate::arguments::json::Json;
use crate::bedrock_content::documents::{rename_duplicate_documents, with_text_when_document_only};
use crate::bedrock_content::error::ContentError;

use super::prelude::{field, json};

fn document_names(contents: &[Json]) -> Vec<Json> {
    contents
        .iter()
        .flat_map(|message| match field(message, "content") {
            Json::List(blocks) => blocks.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                Vec::new()
            }
        })
        .filter_map(|block| {
            block
                .as_object()?
                .get("document")?
                .as_object()?
                .get("name")
                .cloned()
        })
        .collect()
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_document_only_user_message_gets_a_placeholder() {
    let message = json(r#"{"role":"user","content":[{"document":{"name":"doc"}}]}"#);
    assert_eq!(
        with_text_when_document_only(&message).unwrap(),
        json(r#"{"role":"user","content":[{"document":{"name":"doc"}},{"text":"."}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_the_placeholder_precedes_a_trailing_cache_point() {
    let message = json(
        r#"{"role":"user","content":[{"document":{"name":"doc"}},{"cachePoint":{"type":"default"}}]}"#,
    );
    assert_eq!(
        with_text_when_document_only(&message).unwrap(),
        json(
            r#"{"role":"user","content":[{"document":{"name":"doc"}},{"text":"."},{"cachePoint":{"type":"default"}}]}"#
        )
    );
}

#[rstest::rstest]
#[case::assistant_role(r#"{"role":"assistant","content":[{"document":{"name":"doc"}}]}"#)]
#[case::text_part(r#"{"role":"user","content":[{"document":{"name":"doc"}},{"text":"explain"}]}"#)]
#[case::null_text_part(r#"{"role":"user","content":[{"document":{"name":"doc"}},{"text":null}]}"#)]
#[case::no_document(r#"{"role":"user","content":[{"text":"hi"}]}"#)]
#[case::empty_content(r#"{"role":"user","content":[]}"#)]
fn bedrock_content_behavior_tests_behavior_messages_without_a_document_only_turn_are_unchanged(
    #[case] message_json: &str,
) {
    let message = json(message_json);
    assert_eq!(with_text_when_document_only(&message).unwrap(), message);
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_null_document_key_still_needs_text() {
    let message = json(r#"{"role":"user","content":[{"document":null}]}"#);
    assert_eq!(
        with_text_when_document_only(&message).unwrap(),
        json(r#"{"role":"user","content":[{"document":null},{"text":"."}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_rewritten_message_keeps_only_role_and_content() {
    let message = json(r#"{"role":"user","name":"bob","content":[{"document":{"name":"doc"}}]}"#);
    assert_eq!(
        with_text_when_document_only(&message).unwrap(),
        json(r#"{"role":"user","content":[{"document":{"name":"doc"}},{"text":"."}]}"#)
    );
}

#[rstest::rstest]
#[case::missing_content(r#"{"role":"user"}"#, ContentError::MessageWithoutContent)]
#[case::null_content(
    r#"{"role":"user","content":null}"#,
    ContentError::MessageContentNotList
)]
#[case::string_content(
    r#"{"role":"user","content":"hi"}"#,
    ContentError::MessageContentNotList
)]
#[case::missing_role(
    r#"{"content":[{"document":{"name":"doc"}}]}"#,
    ContentError::MessageWithoutRole
)]
#[case::not_an_object("5", ContentError::MessageNotObject)]
fn bedrock_content_behavior_tests_behavior_document_insertion_failures(
    #[case] message_json: &str,
    #[case] expected: ContentError,
) {
    let message = json(message_json);
    assert_eq!(with_text_when_document_only(&message), Err(expected));
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_non_object_block_is_rejected() {
    let message = json(r#"{"role":"user","content":[{"document":{"name":"doc"}},"bad"]}"#);
    assert_eq!(
        with_text_when_document_only(&message),
        Err(ContentError::BlockNotObject { index: 1 })
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_duplicate_document_names_keep_the_first_name() {
    let contents = vec![
        json(r#"{"role":"user","content":[{"document":{"name":"report"}}]}"#),
        json(r#"{"role":"assistant","content":[{"document":{"name":"report"}}]}"#),
        json(r#"{"role":"user","content":[{"document":{"name":"report"}}]}"#),
    ];
    let renamed = rename_duplicate_documents(&contents).unwrap();
    assert_eq!(
        document_names(&renamed),
        ["report", "report_2", "report_3"].map(|name| json(&format!(r#""{name}""#)))
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_rename_skips_an_organic_suffix() {
    let contents = vec![json(
        r#"{"role":"user","content":[{"document":{"name":"report"}},{"document":{"name":"report_2"}},{"document":{"name":"report"}}]}"#,
    )];
    let renamed = rename_duplicate_documents(&contents).unwrap();
    assert_eq!(
        document_names(&renamed),
        ["report", "report_2", "report_3"].map(|name| json(&format!(r#""{name}""#)))
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_rename_counts_occurrences_across_messages() {
    let contents = vec![
        json(r#"{"role":"user","content":[{"document":{"name":"a"}}]}"#),
        json(r#"{"role":"user","content":[{"document":{"name":"a"}}]}"#),
        json(r#"{"role":"user","content":[{"document":{"name":"a"}}]}"#),
        json(r#"{"role":"user","content":[{"document":{"name":"a"}}]}"#),
    ];
    let renamed = rename_duplicate_documents(&contents).unwrap();
    assert_eq!(
        document_names(&renamed),
        ["a", "a_2", "a_3", "a_4"].map(|name| json(&format!(r#""{name}""#)))
    );
}

#[rstest::rstest]
#[case::missing_name(r#"{"document":{}}"#)]
#[case::null_name(r#"{"document":{"name":null}}"#)]
#[case::empty_name(r#"{"document":{"name":""}}"#)]
fn bedrock_content_behavior_tests_behavior_documents_without_a_name_are_left_alone(
    #[case] document_block: &str,
) {
    let contents = vec![json(&format!(
        r#"{{"role":"user","content":[{document_block}]}}"#
    ))];
    let renamed = rename_duplicate_documents(&contents).unwrap();
    assert_eq!(renamed, contents);
}

#[test]
fn bedrock_content_behavior_tests_behavior_two_empty_names_are_left_alone() {
    let contents = vec![json(
        r#"{"role":"user","content":[{"document":{"name":""}},{"document":{"name":""}}]}"#,
    )];
    assert_eq!(rename_duplicate_documents(&contents).unwrap(), contents);
}

#[test]
fn bedrock_content_behavior_tests_behavior_non_object_documents_are_left_alone() {
    let contents = vec![json(
        r#"{"role":"user","content":[{"document":null},{"document":"text"},{"document":{"name":"same"}},{"document":{"name":"same"}}]}"#,
    )];
    let renamed = rename_duplicate_documents(&contents).unwrap();
    assert_eq!(
        renamed,
        [json(
            r#"{"role":"user","content":[{"document":null},{"document":"text"},{"document":{"name":"same"}},{"document":{"name":"same_2"}}]}"#
        )]
    );
}

#[rstest::rstest]
#[case::missing_content(r#"{"role":"user"}"#)]
#[case::null_content(r#"{"role":"user","content":null}"#)]
#[case::empty_content(r#"{"role":"user","content":[]}"#)]
#[case::empty_string_content(r#"{"role":"user","content":""}"#)]
fn bedrock_content_behavior_tests_behavior_messages_without_blocks_are_copied_unchanged(
    #[case] message_json: &str,
) {
    let contents = vec![json(message_json)];
    let renamed = rename_duplicate_documents(&contents).unwrap();
    assert_eq!(renamed, contents);
}

#[rstest::rstest]
#[case::not_an_object("5", ContentError::MessageAtNotObject { index: 0 })]
#[case::string_content(
    r#"{"role":"user","content":"hi"}"#,
    ContentError::MessageContentNotListAt { index: 0 }
)]
fn bedrock_content_behavior_tests_behavior_document_renaming_message_failures(
    #[case] message_json: &str,
    #[case] expected: ContentError,
) {
    let contents = vec![json(message_json)];
    assert_eq!(rename_duplicate_documents(&contents), Err(expected));
}

#[test]
fn bedrock_content_behavior_tests_behavior_document_renaming_rejects_a_non_object_block() {
    let contents = vec![json(r#"{"role":"user","content":["bad"]}"#)];
    assert_eq!(
        rename_duplicate_documents(&contents),
        Err(ContentError::BlockNotObject { index: 0 })
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_document_renaming_rejects_a_non_text_name() {
    let contents = vec![json(
        r#"{"role":"user","content":[{"document":{"name":7}}]}"#,
    )];
    assert_eq!(
        rename_duplicate_documents(&contents),
        Err(ContentError::DocumentNameNotText)
    );
}
