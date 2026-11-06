use super::*;
use crate::arguments::json::Object;
use crate::file_ids::responses::{
    ResponsesContent, ResponsesContentPart, ResponsesInput, ResponsesItem,
    update_responses_file_ids,
};
use rstest::rstest;

fn input_file(file_id: &str) -> ResponsesInput {
    let part = ResponsesContentPart {
        kind: Some("input_file".to_owned()),
        fields: Object::from_iter([("file_id".to_owned(), Json::Text(file_id.to_owned()))]),
    };
    ResponsesInput::Items(vec![ResponsesItem {
        content: Some(ResponsesContent::Parts(vec![part])),
        fields: Object::from_iter([("role".to_owned(), Json::Text("user".to_owned()))]),
    }])
}

fn updated_file_id(input: &ResponsesInput) -> Option<String> {
    let Json::List(items) = serialized(input) else {
        return None;
    };
    let Json::Object(item) = items.first()? else {
        return None;
    };
    let Json::List(parts) = item.get("content")? else {
        return None;
    };
    let Json::Object(part) = parts.first()? else {
        return None;
    };
    part.get("file_id")
        .and_then(Json::as_str)
        .map(str::to_owned)
}

#[rstest]
#[case::mapped_file_id(
    "file-mapped",
    Some(DEPLOYMENT),
    Some(("file-mapped", "provider-file-9")),
    DEPLOYMENT,
    "provider-file-9"
)]
#[case::encoded_unified_id(FILE_UNIFIED, None, None, DEPLOYMENT, "file-xyz")]
#[case::unified_id_without_an_output_field(
    FILE_UNIFIED_NO_OUTPUT,
    None,
    None,
    DEPLOYMENT,
    FILE_UNIFIED_NO_OUTPUT
)]
#[case::model_embedded_id(FILE_EMBEDDED, None, None, DEPLOYMENT, "file-abc123")]
#[case::unmapped_id_is_preserved("file-plain", Some(DEPLOYMENT), None, DEPLOYMENT, "file-plain")]
#[case::empty_file_id_is_preserved(
    "",
    Some(DEPLOYMENT),
    Some(("", "provider-file-9")),
    DEPLOYMENT,
    ""
)]
#[case::missing_model_id_skips_the_mapping(
    FILE_UNIFIED,
    None,
    Some((FILE_UNIFIED, "provider-mapped")),
    DEPLOYMENT,
    "file-xyz"
)]
#[case::mapping_for_another_model_keeps_the_id(
    FILE_UNIFIED,
    Some(DEPLOYMENT),
    Some((FILE_UNIFIED, "provider-mapped")),
    "deployment-2",
    FILE_UNIFIED
)]
fn model_file_ids_behavior_tests_behavior(
    #[case] file_id: &str,
    #[case] model_id: Option<&str>,
    #[case] mapped: Option<(&str, &str)>,
    #[case] mapping_model: &str,
    #[case] expected: &str,
) {
    let mapping = match mapped {
        Some((key, provider)) => mapping(&format!(
            r#"{{"{key}":{{"{mapping_model}":"{provider}"}}}}"#
        )),
        None => FileIdMapping::default(),
    };
    let updated = update_responses_file_ids(&input_file(file_id), model_id, &mapping);
    assert_eq!(updated_file_id(&updated).as_deref(), Some(expected));
}
