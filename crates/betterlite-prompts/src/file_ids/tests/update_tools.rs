use super::*;
use crate::arguments::json::Object;
use crate::file_ids::tools::{ResponsesTool, update_tools_with_file_ids};
use rstest::rstest;

fn code_interpreter(file_ids: &[&str]) -> Vec<ResponsesTool> {
    let ids = file_ids
        .iter()
        .map(|id| Json::Text((*id).to_owned()))
        .collect();
    let container = Object::from_iter([
        ("type".to_owned(), Json::Text("auto".to_owned())),
        ("file_ids".to_owned(), Json::List(ids)),
    ]);
    vec![ResponsesTool {
        kind: Some("code_interpreter".to_owned()),
        fields: Object::from_iter([("container".to_owned(), Json::Object(container))]),
    }]
}

fn container_file_ids(tools: &[ResponsesTool]) -> Vec<&str> {
    let Some(Json::Object(container)) = tools.first().and_then(|tool| tool.fields.get("container"))
    else {
        panic!("the code_interpreter tool keeps its container object");
    };
    assert_eq!(
        container.get("type").and_then(Json::as_str),
        Some("auto"),
        "the container keeps its type"
    );
    let Some(Json::List(ids)) = container.get("file_ids") else {
        panic!("the container keeps its file_ids list");
    };
    ids.iter().filter_map(Json::as_str).collect()
}

#[rstest]
#[case::mapped_and_unmapped_ids(
    &["file-mapped", "file-plain"],
    Some(DEPLOYMENT),
    Some(("file-mapped", "provider-file-9")),
    DEPLOYMENT,
    &["provider-file-9", "file-plain"]
)]
#[case::missing_model_id_keeps_every_id(
    &["file-mapped"],
    None,
    Some(("file-mapped", "provider-file-9")),
    DEPLOYMENT,
    &["file-mapped"]
)]
#[case::mapping_for_another_model_keeps_every_id(
    &["file-mapped"],
    Some(DEPLOYMENT),
    Some(("file-mapped", "provider-file-9")),
    "deployment-2",
    &["file-mapped"]
)]
#[case::empty_mapping_keeps_every_id(
    &["file-mapped"],
    Some(DEPLOYMENT),
    None,
    DEPLOYMENT,
    &["file-mapped"]
)]
fn model_file_ids_behavior_tests_behavior(
    #[case] input: &[&str],
    #[case] model_id: Option<&str>,
    #[case] mapped: Option<(&str, &str)>,
    #[case] mapping_model: &str,
    #[case] expected: &[&str],
) {
    let mapping = match mapped {
        Some((key, provider)) => mapping(&format!(
            r#"{{"{key}":{{"{mapping_model}":"{provider}"}}}}"#
        )),
        None => FileIdMapping::default(),
    };
    let updated = update_tools_with_file_ids(Some(&code_interpreter(input)), model_id, &mapping)
        .expect("the tools stay within the limits")
        .expect("the tool list is present");
    assert_eq!(container_file_ids(&updated), expected);
}
