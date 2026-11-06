use super::*;
use crate::arguments::json::Object;
use crate::file_ids::tools::{ResponsesTool, decode_vector_store_ids, update_tools_with_file_ids};
use rstest::rstest;

fn file_search(vector_ids: &[&str]) -> Vec<ResponsesTool> {
    let ids = vector_ids
        .iter()
        .map(|id| Json::Text((*id).to_owned()))
        .collect();
    vec![ResponsesTool {
        kind: Some("file_search".to_owned()),
        fields: Object::from_iter([("vector_store_ids".to_owned(), Json::List(ids))]),
    }]
}

fn vector_ids(tools: &[ResponsesTool]) -> Vec<&str> {
    let Some(Json::List(ids)) = tools
        .first()
        .and_then(|tool| tool.fields.get("vector_store_ids"))
    else {
        panic!("the file_search tool keeps its vector_store_ids list");
    };
    ids.iter().filter_map(Json::as_str).collect()
}

#[rstest]
#[case::encoded_vector_id(&[VECTOR_UNIFIED], &["vs_abc123"])]
#[case::plain_vector_id(&["vs_plain"], &["vs_plain"])]
#[case::mixed_ids(&[VECTOR_UNIFIED, "vs_plain"], &["vs_abc123", "vs_plain"])]
fn model_file_ids_behavior_tests_behavior(#[case] input: &[&str], #[case] expected: &[&str]) {
    let tools = file_search(input);
    let decoded = decode_vector_store_ids(Some(&tools)).expect("a tool list decodes");
    assert_eq!(vector_ids(&decoded), expected);
    let updated = update_tools_with_file_ids(Some(&tools), None, &FileIdMapping::default())
        .expect("a tool list decodes");
    assert_eq!(vector_ids(&updated), expected);
}
