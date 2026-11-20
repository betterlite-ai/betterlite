use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::file_ids::limits::FileIdsError;
use betterlite_prompts::file_ids::mapping::FileIdMapping;
use betterlite_prompts::file_ids::responses::{ResponsesContent, ResponsesInput, ResponsesItem};
use betterlite_prompts::file_ids::tools::ResponsesTool;
use rstest::rstest;

use super::json;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::file_ids::{
    FileMappingError, apply_managed_file_id_mapping, model_info_id,
};

fn mapping(text: &str) -> FileIdMapping {
    serde_json::from_str(text).expect("the mapping parses")
}

fn response_input(text: &str) -> ResponsesInput {
    serde_json::from_str(text).expect("the response input parses")
}

fn response_tools(text: &str) -> Vec<ResponsesTool> {
    serde_json::from_str(text).expect("the tools parse")
}

const FILE_INPUT: &str =
    r#"[{"type":"message","role":"user","content":[{"type":"input_file","file_id":"file-abc"}]}]"#;
const MAPPED_FILE_INPUT: &str = r#"[{"type":"message","role":"user","content":[{"type":"input_file","file_id":"file-provider"}]}]"#;

#[test]
fn a_mapped_file_id_uses_the_deployment_provider_id() {
    let input = response_input(FILE_INPUT);
    let resolved = mapping(r#"{"file-abc":{"dep-1":"file-provider"}}"#);
    let model_info = json(r#"{"id":"dep-1"}"#);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_managed_file_id_mapping(
        &input,
        None,
        Some(&model_info),
        Some(&resolved),
        &mut local_vars,
    )
    .expect("the mapping applies");
    assert_eq!(outcome.input, response_input(MAPPED_FILE_INPUT));
    assert_eq!(outcome.tools, None);
    assert_eq!(local_vars.get("input"), Some(&json(MAPPED_FILE_INPUT)));
    assert!(!local_vars.contains_key("tools"));
}

#[rstest]
#[case::absent(None)]
#[case::no_id(Some(r#"{"supported_endpoints":[]}"#))]
fn without_a_deployment_id_the_file_id_is_unchanged(#[case] model_info: Option<&str>) {
    let input = response_input(FILE_INPUT);
    let resolved = mapping(r#"{"file-abc":{"dep-1":"file-provider"}}"#);
    let info = model_info.map(json);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_managed_file_id_mapping(
        &input,
        None,
        info.as_ref(),
        Some(&resolved),
        &mut local_vars,
    )
    .expect("the mapping applies");
    assert_eq!(outcome.input, input);
    assert_eq!(local_vars.get("input"), Some(&json(FILE_INPUT)));
}

#[test]
fn without_a_mapping_the_input_still_reaches_the_locals() {
    let input = response_input(FILE_INPUT);
    let model_info = json(r#"{"id":"dep-1"}"#);
    let mut local_vars = ParameterMap::new();
    let outcome =
        apply_managed_file_id_mapping(&input, None, Some(&model_info), None, &mut local_vars)
            .expect("the mapping applies");
    assert_eq!(outcome.input, input);
    assert_eq!(local_vars.get("input"), Some(&json(FILE_INPUT)));
}

#[test]
fn a_text_input_maps_nothing() {
    let input = response_input(r#""hello""#);
    let model_info = json(r#"{"id":"dep-1"}"#);
    let mut local_vars = ParameterMap::new();
    let outcome =
        apply_managed_file_id_mapping(&input, None, Some(&model_info), None, &mut local_vars)
            .expect("the mapping applies");
    assert_eq!(outcome.input, response_input(r#""hello""#));
    assert_eq!(local_vars.get("input"), Some(&json(r#""hello""#)));
}

#[rstest]
#[case::id(Some(r#"{"id":"dep-1"}"#), Some("dep-1"))]
#[case::number_id(Some(r#"{"id":7}"#), None)]
#[case::missing_id(Some("{}"), None)]
#[case::null_info(Some("null"), None)]
#[case::text_info(Some(r#""dep-1""#), None)]
fn only_a_text_id_names_the_deployment(
    #[case] model_info: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let info = model_info.map(json);
    assert_eq!(model_info_id(info.as_ref()).as_deref(), expected);
}

#[test]
fn a_code_interpreter_tool_maps_its_container_file_ids() {
    let input = response_input("[]");
    let tools = response_tools(
        r#"[{"type":"code_interpreter","container":{"file_ids":["file-abc","file-other"]}}]"#,
    );
    let resolved = mapping(r#"{"file-abc":{"dep-1":"file-provider"}}"#);
    let model_info = json(r#"{"id":"dep-1"}"#);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_managed_file_id_mapping(
        &input,
        Some(&tools),
        Some(&model_info),
        Some(&resolved),
        &mut local_vars,
    )
    .expect("the mapping applies");
    let expected_tools = response_tools(
        r#"[{"type":"code_interpreter","container":{"file_ids":["file-provider","file-other"]}}]"#,
    );
    assert_eq!(outcome.tools, Some(expected_tools));
    assert_eq!(
        local_vars.get("tools"),
        Some(&json(
            r#"[{"type":"code_interpreter","container":{"file_ids":["file-provider","file-other"]}}]"#
        ))
    );
}

#[test]
fn tools_reach_the_locals_even_without_deployment_metadata() {
    let input = response_input("[]");
    let tools =
        response_tools(r#"[{"type":"code_interpreter","container":{"file_ids":["file-abc"]}}]"#);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_managed_file_id_mapping(&input, Some(&tools), None, None, &mut local_vars)
        .expect("the mapping applies");
    assert_eq!(outcome.tools, Some(tools.clone()));
    assert_eq!(
        local_vars.get("tools"),
        Some(&json(
            r#"[{"type":"code_interpreter","container":{"file_ids":["file-abc"]}}]"#
        ))
    );
}

#[test]
fn a_plain_file_search_id_passes_through_and_tools_reach_the_locals() {
    let input = response_input("[]");
    let tools = response_tools(r#"[{"type":"file_search","vector_store_ids":["vs-plain"]}]"#);
    let model_info = json(r#"{"id":"dep-1"}"#);
    let resolved = mapping(r#"{"file-abc":{"dep-1":"file-provider"}}"#);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_managed_file_id_mapping(
        &input,
        Some(&tools),
        Some(&model_info),
        Some(&resolved),
        &mut local_vars,
    )
    .expect("the mapping applies");
    assert_eq!(outcome.tools, Some(tools.clone()));
    assert_eq!(
        local_vars.get("tools"),
        Some(&json(
            r#"[{"type":"file_search","vector_store_ids":["vs-plain"]}]"#
        ))
    );
}

#[test]
fn an_empty_tool_list_is_skipped() {
    let input = response_input("[]");
    let tools: Vec<ResponsesTool> = Vec::new();
    let model_info = json(r#"{"id":"dep-1"}"#);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_managed_file_id_mapping(
        &input,
        Some(&tools),
        Some(&model_info),
        None,
        &mut local_vars,
    )
    .expect("the mapping applies");
    assert_eq!(outcome.tools, Some(Vec::new()));
    assert!(!local_vars.contains_key("tools"));
}

#[test]
fn a_too_large_input_is_a_file_ids_error() {
    let parts = (0..257_u32)
        .map(|_| json(r#"{"type":"input_text","text":"x"}"#))
        .collect();
    let input = ResponsesInput::Items(vec![ResponsesItem {
        content: Some(ResponsesContent::Parts(parts)),
        fields: Object::new(),
    }]);
    let mut local_vars = ParameterMap::new();
    let error = apply_managed_file_id_mapping(&input, None, None, None, &mut local_vars)
        .expect_err("the input exceeds the part limit");
    assert!(matches!(
        error,
        FileMappingError::FileIds(FileIdsError::TooManyInputParts { .. })
    ));
}
