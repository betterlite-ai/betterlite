use super::*;
use crate::arguments::json::Object;
use crate::file_ids::messages::update_messages_with_file_ids;
use crate::message_content::{Content, ContentPart};
use rstest::rstest;

fn file_message(file_id: &str, existing_format: Option<&str>) -> Message {
    let mut file = Object::from_iter([("file_id".to_owned(), Json::Text(file_id.to_owned()))]);
    if let Some(format) = existing_format {
        file.insert("format".to_owned(), Json::Text(format.to_owned()));
    }
    Message {
        role: "user".to_owned(),
        content: Some(Content::Parts(vec![ContentPart {
            kind: "file".to_owned(),
            fields: Object::from_iter([("file".to_owned(), Json::Object(file))]),
        }])),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

#[rstest]
#[case::mapped_id_replaces_the_file_id(
    "file-mapped",
    None,
    Some(DEPLOYMENT),
    Some(("file-mapped", DEPLOYMENT, "provider-file-9")),
    ("provider-file-9", None)
)]
#[case::existing_format_is_kept(
    "file-mapped",
    Some("application/pdf"),
    Some(DEPLOYMENT),
    Some(("file-mapped", DEPLOYMENT, "provider-file-9")),
    ("provider-file-9", Some("application/pdf"))
)]
#[case::existing_format_wins_over_the_derived_one(
    FILE_UNIFIED,
    Some("application/pdf"),
    None,
    None,
    ("file-xyz", Some("application/pdf"))
)]
#[case::empty_derived_format_is_not_written(
    "litellm_proxy:;unified_id,abc",
    None,
    None,
    None,
    ("litellm_proxy:;unified_id,abc", None)
)]
#[case::empty_mapping_value_falls_back_to_the_id(
    "file-mapped",
    None,
    Some(DEPLOYMENT),
    Some(("file-mapped", DEPLOYMENT, "")),
    ("file-mapped", None)
)]
#[case::empty_output_file_id_falls_back_to_the_id(
    FILE_UNIFIED_EMPTY_OUTPUT,
    None,
    None,
    None,
    (FILE_UNIFIED_EMPTY_OUTPUT, Some("file.x"))
)]
#[case::unmapped_id_is_preserved(
    "file-plain",
    None,
    Some(DEPLOYMENT),
    None,
    ("file-plain", None)
)]
#[case::mapping_for_another_model_is_ignored(
    "file-mapped",
    None,
    Some(DEPLOYMENT),
    Some(("file-mapped", "deployment-2", "provider-file-9")),
    ("file-mapped", None)
)]
#[case::encoded_unified_id_yields_its_output_file_id(
    FILE_UNIFIED,
    None,
    None,
    None,
    ("file-xyz", Some("file.abc123"))
)]
#[case::encoded_unified_id_without_an_output_field_is_kept(
    FILE_UNIFIED_NO_OUTPUT,
    None,
    None,
    None,
    (FILE_UNIFIED_NO_OUTPUT, Some("file.abc123"))
)]
#[case::model_embedded_id_yields_the_original_id(
    FILE_EMBEDDED,
    None,
    None,
    None,
    ("file-abc123", None)
)]
#[case::mapping_wins_over_an_encoded_unified_id(
    FILE_UNIFIED,
    None,
    Some(DEPLOYMENT),
    Some((FILE_UNIFIED, DEPLOYMENT, "provider-mapped")),
    ("provider-mapped", Some("file.abc123"))
)]
#[case::missing_model_id_skips_the_mapping(
    FILE_EMBEDDED,
    None,
    None,
    Some((FILE_EMBEDDED, DEPLOYMENT, "provider-file-9")),
    ("file-abc123", None)
)]
#[case::empty_file_id_is_left_alone(
    "",
    None,
    Some(DEPLOYMENT),
    Some(("", DEPLOYMENT, "provider-file-9")),
    ("", None)
)]
fn model_file_ids_behavior_tests_behavior(
    #[case] file_id: &str,
    #[case] existing_format: Option<&str>,
    #[case] model_id: Option<&str>,
    #[case] mapped: Option<(&str, &str, &str)>,
    #[case] expected: (&str, Option<&str>),
) {
    let mapping = match mapped {
        Some((key, mapping_model, provider)) => mapping(&format!(
            r#"{{"{key}":{{"{mapping_model}":"{provider}"}}}}"#
        )),
        None => FileIdMapping::default(),
    };
    let mut updated = vec![file_message(file_id, existing_format)];
    update_messages_with_file_ids(&mut updated, model_id, &mapping)
        .expect("the message list stays within the limits");
    let Some(Content::Parts(parts)) = updated.first().and_then(|message| message.content.as_ref())
    else {
        panic!("the user message keeps its content parts");
    };
    let Some(Json::Object(file)) = parts.first().and_then(|part| part.fields.get("file")) else {
        panic!("the file part keeps its file object");
    };
    let (expected_id, expected_format) = expected;
    assert_eq!(
        file.get("file_id").and_then(Json::as_str),
        Some(expected_id)
    );
    assert_eq!(file.get("format").and_then(Json::as_str), expected_format);
}
