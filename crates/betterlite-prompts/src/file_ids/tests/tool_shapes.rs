use super::*;
use crate::file_ids::tools::{ResponsesTool, decode_vector_store_ids, update_tools_with_file_ids};
use rstest::rstest;

fn tools(text: &str) -> Option<Vec<ResponsesTool>> {
    serde_json::from_str(text).expect("test tools decode")
}

#[rstest]
#[case::absent_tools("null", "null")]
#[case::empty_tools("[]", "[]")]
#[case::vector_store_ids_not_a_list(
    r#"[{"type":"file_search","vector_store_ids":"vs_not_a_list"}]"#,
    r#"[{"type":"file_search","vector_store_ids":"vs_not_a_list"}]"#
)]
#[case::file_search_without_vector_store_ids(
    r#"[{"type":"file_search"}]"#,
    r#"[{"type":"file_search"}]"#
)]
#[case::non_string_vector_store_ids(
    r#"[{"type":"file_search","vector_store_ids":[5,true,null]}]"#,
    r#"[{"type":"file_search","vector_store_ids":[5,true,null]}]"#
)]
#[case::code_interpreter_is_not_decoded(
    r#"[{"type":"code_interpreter","container":{"type":"auto","file_ids":["file-plain"]}}]"#,
    r#"[{"type":"code_interpreter","container":{"type":"auto","file_ids":["file-plain"]}}]"#
)]
#[case::function_tool_is_preserved(
    r#"[{"type":"function","name":"lookup"}]"#,
    r#"[{"type":"function","name":"lookup"}]"#
)]
#[case::container_without_file_ids(
    r#"[{"type":"code_interpreter","container":{"type":"auto"}}]"#,
    r#"[{"type":"code_interpreter","container":{"type":"auto"}}]"#
)]
#[case::container_file_ids_not_a_list(
    r#"[{"type":"code_interpreter","container":{"file_ids":"nope"}}]"#,
    r#"[{"type":"code_interpreter","container":{"file_ids":"nope"}}]"#
)]
#[case::code_interpreter_without_a_container(
    r#"[{"type":"code_interpreter"}]"#,
    r#"[{"type":"code_interpreter"}]"#
)]
fn model_file_ids_behavior_tests_behavior(#[case] input: &str, #[case] expected: &str) {
    let tools = tools(input);
    let decoded = decode_vector_store_ids(tools.as_deref());
    assert_eq!(serialized(&decoded), json(expected));
    let updated = update_tools_with_file_ids(
        tools.as_deref(),
        Some(DEPLOYMENT),
        &FileIdMapping::default(),
    );
    assert_eq!(serialized(&updated), json(expected));
}
