#![cfg(test)]

use rstest::rstest;

use crate::shadow_requests::Object;

use super::request_has_hosted_web_search;

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::empty_object(r#"{"web_search_options":{}}"#)]
#[case::populated(r#"{"web_search_options":{"search_context_size":"low"}}"#)]
#[case::empty_array(r#"{"web_search_options":[]}"#)]
#[case::empty_string(r#"{"web_search_options":""}"#)]
fn a_non_null_web_search_options_is_hosted(#[case] source: &str) {
    assert!(request_has_hosted_web_search(&object(source)));
}

#[test]
fn a_null_web_search_options_is_not_hosted() {
    assert!(!request_has_hosted_web_search(&object(
        r#"{"web_search_options":null}"#
    )));
}

#[rstest]
#[case::web_search("web_search")]
#[case::preview("web_search_preview")]
#[case::preview_dated("web_search_preview_2025_03_11")]
#[case::other_prefixed_type("web_search_custom")]
fn a_web_search_tool_type_is_hosted(#[case] tool_type: &str) {
    let source = format!(r#"{{"tools":[{{"type":"{tool_type}"}}]}}"#);
    assert!(request_has_hosted_web_search(&object(&source)));
}

#[rstest]
#[case::litellm_web_search("litellm_web_search")]
#[case::other_function("lookup")]
fn a_function_tool_is_never_hosted(#[case] name: &str) {
    let source = format!(r#"{{"tools":[{{"type":"function","name":"{name}"}}]}}"#);
    assert!(!request_has_hosted_web_search(&object(&source)));
}

#[test]
fn a_function_tool_does_not_hide_a_later_hosted_tool() {
    assert!(request_has_hosted_web_search(&object(
        r#"{"tools":[{"type":"function","name":"litellm_web_search"},{"type":"web_search"}]}"#
    )));
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"tools":null}"#)]
#[case::object(r#"{"tools":{}}"#)]
#[case::string(r#"{"tools":"web_search"}"#)]
#[case::number(r#"{"tools":5}"#)]
fn tools_that_are_not_an_array_are_not_hosted(#[case] source: &str) {
    assert!(!request_has_hosted_web_search(&object(source)));
}

#[test]
fn non_object_tools_are_not_hosted() {
    assert!(!request_has_hosted_web_search(&object(
        r#"{"tools":[1,"web_search",null]}"#
    )));
}

#[rstest]
#[case::no_type(r#"{"tools":[{"name":"web_search"}]}"#)]
#[case::null_type(r#"{"tools":[{"type":null}]}"#)]
#[case::number_type(r#"{"tools":[{"type":5}]}"#)]
fn a_tool_without_a_text_type_is_not_hosted(#[case] source: &str) {
    assert!(!request_has_hosted_web_search(&object(source)));
}
