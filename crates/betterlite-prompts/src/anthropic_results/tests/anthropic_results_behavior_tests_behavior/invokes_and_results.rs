use crate::anthropic_results::function_invoke::{InvokeError, to_anthropic_tool_use};
use crate::anthropic_results::server_result::find_server_tool_result;
use crate::arguments::json::Json;

use super::object;

fn list(raw: &str) -> Vec<Json> {
    let Json::List(items) = serde_json::from_str::<Json>(raw).unwrap() else {
        panic!("expected a JSON list");
    };
    items
}

#[test]
fn anthropic_results_behavior_tests_behavior_a_function_call_becomes_a_tool_use() {
    let calls = to_anthropic_tool_use(
        &object(r#"{"name":"weather","arguments":"{\"city\":\"Paris\"}"}"#),
        &|| "fixed-id".to_owned(),
    )
    .unwrap();
    assert_eq!(
        calls,
        vec![object(
            r#"{"type":"tool_use","id":"fixed-id","name":"weather","input":{"city":"Paris"}}"#
        )]
    );
}

#[test]
fn anthropic_results_behavior_tests_behavior_missing_arguments_become_an_empty_object() {
    let calls =
        to_anthropic_tool_use(&object(r#"{"name":"weather"}"#), &|| "fixed-id".to_owned()).unwrap();
    assert_eq!(
        calls,
        vec![object(
            r#"{"type":"tool_use","id":"fixed-id","name":"weather","input":{}}"#
        )]
    );
}

#[test]
fn anthropic_results_behavior_tests_behavior_empty_arguments_become_an_empty_object() {
    let calls = to_anthropic_tool_use(&object(r#"{"name":"weather","arguments":"  "}"#), &|| {
        "fixed-id".to_owned()
    })
    .unwrap();
    assert_eq!(
        calls,
        vec![object(
            r#"{"type":"tool_use","id":"fixed-id","name":"weather","input":{}}"#
        )]
    );
}

#[test]
fn anthropic_results_behavior_tests_behavior_a_missing_name_becomes_an_empty_string() {
    let calls =
        to_anthropic_tool_use(&object(r#"{"arguments":"{}"}"#), &|| "fixed-id".to_owned()).unwrap();
    assert_eq!(
        calls,
        vec![object(
            r#"{"type":"tool_use","id":"fixed-id","name":"","input":{}}"#
        )]
    );
}

#[test]
fn anthropic_results_behavior_tests_behavior_malformed_arguments_are_an_error() {
    let error = to_anthropic_tool_use(
        &object(r#"{"name":"weather","arguments":"not json"}"#),
        &|| "fixed-id".to_owned(),
    )
    .unwrap_err();
    assert!(matches!(error, InvokeError::Arguments(_)));
}

#[test]
fn anthropic_results_behavior_tests_behavior_a_non_string_arguments_field_is_named() {
    let error = to_anthropic_tool_use(&object(r#"{"name":"weather","arguments":5}"#), &|| {
        "fixed-id".to_owned()
    })
    .unwrap_err();
    assert!(matches!(error, InvokeError::Field { field: "arguments" }));
}

#[test]
fn anthropic_results_behavior_tests_behavior_a_non_string_name_field_is_named() {
    let error = to_anthropic_tool_use(&object(r#"{"name":5,"arguments":"{}"}"#), &|| {
        "fixed-id".to_owned()
    })
    .unwrap_err();
    assert!(matches!(error, InvokeError::Field { field: "name" }));
}

#[test]
fn anthropic_results_behavior_tests_behavior_a_web_search_result_is_found() {
    let web = list(r#"[{"tool_use_id":"search-1","result":"a"}]"#);
    assert_eq!(
        find_server_tool_result("search-1", &web, &[]),
        Some(&object(r#"{"tool_use_id":"search-1","result":"a"}"#))
    );
}

#[test]
fn anthropic_results_behavior_tests_behavior_a_tool_result_is_found_after_the_web_search() {
    let web = list(r#"[{"tool_use_id":"other"}]"#);
    let tools = list(r#"[{"tool_use_id":"code-1","result":"b"}]"#);
    assert_eq!(
        find_server_tool_result("code-1", &web, &tools),
        Some(&object(r#"{"tool_use_id":"code-1","result":"b"}"#))
    );
}

#[test]
fn anthropic_results_behavior_tests_behavior_the_web_search_result_comes_first() {
    let web = list(r#"[{"tool_use_id":"shared","result":"web"}]"#);
    let tools = list(r#"[{"tool_use_id":"shared","result":"tool"}]"#);
    assert_eq!(
        find_server_tool_result("shared", &web, &tools),
        Some(&object(r#"{"tool_use_id":"shared","result":"web"}"#))
    );
}

#[test]
fn anthropic_results_behavior_tests_behavior_non_object_candidates_are_skipped() {
    let web = list(r#"[1,"text",{"tool_use_id":"search-2"}]"#);
    assert_eq!(
        find_server_tool_result("search-2", &web, &[]),
        Some(&object(r#"{"tool_use_id":"search-2"}"#))
    );
}

#[test]
fn anthropic_results_behavior_tests_behavior_a_non_string_tool_use_id_never_matches() {
    let web = list(r#"[{"tool_use_id":5}]"#);
    assert_eq!(find_server_tool_result("5", &web, &[]), None);
}

#[test]
fn anthropic_results_behavior_tests_behavior_no_match_returns_none() {
    let web = list(r#"[{"tool_use_id":"search-1"}]"#);
    assert_eq!(find_server_tool_result("missing", &web, &[]), None);
}

#[test]
fn anthropic_results_behavior_tests_behavior_empty_sequences_return_none() {
    assert_eq!(find_server_tool_result("id", &[], &[]), None);
}
