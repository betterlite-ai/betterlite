#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;

use super::{
    get_file_search_tool_call, get_web_search_options, is_file_search_tool_call,
    is_web_search_tool_call, tools_from_kwargs,
};
use crate::decisions::Json;
use crate::tool_signals::error::ToolSignalError;

fn kwargs(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("the kwargs fixture is a mapping")
}

fn fields(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("the tool fixture is a mapping")
}

#[test]
fn an_explicit_empty_options_object_wins_over_the_tools() {
    let options = get_web_search_options(&kwargs(
        r#"{"web_search_options":{},"tools":[{"type":"web_search"}]}"#,
    ))
    .expect("the options construct");
    assert_eq!(options.map(|options| options.fields), Some(IndexMap::new()));
}

#[test]
fn explicit_options_copy_every_field_unchanged() {
    let options = get_web_search_options(&kwargs(
        r#"{"web_search_options":{"search_context_size":"gigantic","extra":{"nested":[1,2]}}}"#,
    ))
    .expect("the options construct")
    .expect("the options are present");
    assert_eq!(
        options.fields,
        fields(r#"{"search_context_size":"gigantic","extra":{"nested":[1,2]}}"#)
    );
}

#[rstest]
#[case::null(r#"{"web_search_options":null,"tools":[{"type":"web_search"}]}"#)]
#[case::list(r#"{"web_search_options":[]}"#)]
#[case::number(r#"{"web_search_options":5}"#)]
#[case::text(r#"{"web_search_options":"web_search"}"#)]
fn a_non_mapping_explicit_options_value_is_an_error(#[case] source: &str) {
    assert_eq!(
        get_web_search_options(&kwargs(source)),
        Err(ToolSignalError::OptionsNotMapping)
    );
}

#[test]
fn no_tools_key_yields_no_options() {
    assert_eq!(get_web_search_options(&kwargs(r#"{}"#)), Ok(None));
}

#[rstest]
#[case::null(r#"{"tools":null}"#)]
#[case::empty_list(r#"{"tools":[]}"#)]
#[case::empty_object(r#"{"tools":{}}"#)]
#[case::zero(r#"{"tools":0}"#)]
#[case::text(r#"{"tools":"web_search"}"#)]
#[case::object(r#"{"tools":{"type":"web_search"}}"#)]
#[case::non_matching_list(r#"{"tools":[{"type":"file_search"}]}"#)]
fn a_truthy_or_empty_tools_value_without_a_match_yields_no_options(#[case] source: &str) {
    assert_eq!(get_web_search_options(&kwargs(source)), Ok(None));
}

#[rstest]
#[case::number(r#"{"tools":5}"#)]
#[case::boolean(r#"{"tools":true}"#)]
fn a_non_iterable_tools_value_is_an_error(#[case] source: &str) {
    assert_eq!(
        get_web_search_options(&kwargs(source)),
        Err(ToolSignalError::ToolsNotIterable)
    );
}

#[test]
fn the_first_matching_tool_in_original_order_wins() {
    let options = get_web_search_options(&kwargs(
        r#"{"tools":[{"type":"other","search_context_size":"high"},
        {"type":"web_search","search_context_size":"low"}]}"#,
    ))
    .expect("the options construct")
    .expect("the first matching tool is present");
    assert_eq!(
        options.fields,
        fields(r#"{"type":"other","search_context_size":"high"}"#)
    );
}

#[test]
fn a_tool_matches_by_type_or_by_the_search_context_key() {
    let options = get_web_search_options(&kwargs(
        r#"{"tools":["plain",{"type":"web_search_preview"},{"type":"web_search"}]}"#,
    ))
    .expect("the options construct")
    .expect("the first matching tool is present");
    assert_eq!(options.fields, fields(r#"{"type":"web_search_preview"}"#));
}

#[test]
fn a_tool_type_alone_is_not_a_match() {
    assert_eq!(
        get_web_search_options(&kwargs(
            r#"{"tools":[{"type":"other"},{"type":"Web_Search"},{"search_context_size":null}]}"#,
        ))
        .expect("the options construct")
        .map(|options| options.fields),
        Some(fields(r#"{"search_context_size":null}"#))
    );
}

#[test]
fn file_search_picks_the_first_file_search_tool_independently() {
    let tool = get_file_search_tool_call(&kwargs(
        r#"{"tools":[{"type":"web_search","search_context_size":"high"},
        {"type":"file_search","vector_store_ids":["vs_1"],"extra":true}]}"#,
    ))
    .expect("the tool constructs")
    .expect("the file search tool is present");
    assert_eq!(
        tool.fields,
        fields(r#"{"type":"file_search","vector_store_ids":["vs_1"],"extra":true}"#)
    );
}

#[test]
fn file_search_ignores_non_matching_and_non_object_tools() {
    assert_eq!(
        get_file_search_tool_call(&kwargs(r#"{"tools":["file_search",{"type":"other"}]}"#))
            .expect("the tools scan"),
        None
    );
}

#[test]
fn tools_from_kwargs_returns_the_original_list_for_any_tool_type() {
    let kwargs = kwargs(r#"{"tools":[{"type":"file_search"},{"type":"web_search"}]}"#);
    let preview = tools_from_kwargs(&kwargs, "web_search_preview");
    let file_search = tools_from_kwargs(&kwargs, "file_search");
    assert_eq!(preview, file_search);
    assert!(preview.is_some());
}

#[test]
fn tools_from_kwargs_preserves_an_explicit_null() {
    assert_eq!(
        tools_from_kwargs(&kwargs(r#"{"tools":null}"#), "web_search"),
        Some(&Json::Null)
    );
    assert_eq!(tools_from_kwargs(&kwargs(r#"{}"#), "web_search"), None);
}

#[rstest]
#[case::preview(r#"{"type":"web_search_preview"}"#, true)]
#[case::search(r#"{"type":"web_search"}"#, true)]
#[case::context_only(r#"{"search_context_size":"high"}"#, true)]
#[case::other(r#"{"type":"other"}"#, false)]
#[case::null_type(r#"{"type":null}"#, false)]
#[case::numeric_type(r#"{"type":5}"#, false)]
#[case::similar_type(r#"{"type":"web_search_preview_2025"}"#, false)]
fn web_search_tool_detection_follows_the_source(#[case] tool: &str, #[case] expected: bool) {
    assert_eq!(is_web_search_tool_call(&fields(tool)), expected);
}

#[rstest]
#[case::file_search(r#"{"type":"file_search"}"#, true)]
#[case::other(r#"{"type":"web_search"}"#, false)]
#[case::null_type(r#"{"type":null}"#, false)]
fn file_search_tool_detection_follows_the_source(#[case] tool: &str, #[case] expected: bool) {
    assert_eq!(is_file_search_tool_call(&fields(tool)), expected);
}
