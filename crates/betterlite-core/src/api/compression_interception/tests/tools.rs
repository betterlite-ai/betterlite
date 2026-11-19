#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{list, number, object, text};
use crate::api::compression_interception::tools::{
    CONTENT_RETRIEVE_TOOL_NAME, has_retrieval_tool, merge_tools,
};

fn function_tool(name: Json) -> Json {
    object(vec![
        ("type", text("function")),
        ("function", object(vec![("name", name)])),
    ])
}

fn custom_tool(name: Json) -> Json {
    object(vec![("type", text("custom")), ("name", name)])
}

fn named(name: &str) -> Json {
    text(name)
}

#[rstest]
#[case::absent(None)]
#[case::null(Some(Json::Null))]
#[case::text(Some(text("tools")))]
#[case::number(Some(number(7)))]
#[case::object(Some(object(vec![("type", text("function"))])))]
fn a_non_list_tools_value_has_no_retrieval_tool(#[case] tools: Option<Json>) {
    assert!(!has_retrieval_tool(tools.as_ref()));
}

#[test]
fn an_empty_list_has_no_retrieval_tool() {
    assert!(!has_retrieval_tool(Some(&list(Vec::new()))));
}

#[test]
fn a_function_tool_with_the_retrieval_name_is_found() {
    let tools = list(vec![function_tool(named(CONTENT_RETRIEVE_TOOL_NAME))]);
    assert!(has_retrieval_tool(Some(&tools)));
}

#[test]
fn a_function_tool_with_another_name_is_ignored() {
    let tools = list(vec![function_tool(named("search"))]);
    assert!(!has_retrieval_tool(Some(&tools)));
}

#[test]
fn a_function_tool_without_a_function_object_is_ignored() {
    let tools = list(vec![object(vec![("type", text("function"))])]);
    assert!(!has_retrieval_tool(Some(&tools)));
}

#[test]
fn a_function_tool_whose_function_lacks_a_name_is_ignored() {
    let tools = list(vec![object(vec![
        ("type", text("function")),
        ("function", object(Vec::new())),
    ])]);
    assert!(!has_retrieval_tool(Some(&tools)));
}

#[test]
fn a_function_tool_whose_function_is_not_an_object_is_ignored() {
    let tools = list(vec![object(vec![
        ("type", text("function")),
        ("function", text(CONTENT_RETRIEVE_TOOL_NAME)),
    ])]);
    assert!(!has_retrieval_tool(Some(&tools)));
}

#[test]
fn a_custom_tool_with_the_retrieval_name_is_found() {
    let tools = list(vec![custom_tool(named(CONTENT_RETRIEVE_TOOL_NAME))]);
    assert!(has_retrieval_tool(Some(&tools)));
}

#[test]
fn a_custom_tool_with_another_name_is_ignored() {
    let tools = list(vec![custom_tool(named("search"))]);
    assert!(!has_retrieval_tool(Some(&tools)));
}

#[test]
fn a_bare_name_without_a_type_is_ignored() {
    let tools = list(vec![object(vec![(
        "name",
        text(CONTENT_RETRIEVE_TOOL_NAME),
    )])]);
    assert!(!has_retrieval_tool(Some(&tools)));
}

#[test]
fn a_tool_that_is_not_an_object_is_ignored() {
    let tools = list(vec![text(CONTENT_RETRIEVE_TOOL_NAME)]);
    assert!(!has_retrieval_tool(Some(&tools)));
}

#[test]
fn merging_appends_compressed_tools_to_an_existing_list() {
    let existing = vec![custom_tool(named("search"))];
    let compressed = vec![custom_tool(named(CONTENT_RETRIEVE_TOOL_NAME))];
    assert_eq!(
        merge_tools(&existing, &compressed),
        vec![
            custom_tool(named("search")),
            custom_tool(named(CONTENT_RETRIEVE_TOOL_NAME)),
        ]
    );
}

#[test]
fn merging_without_existing_tools_returns_the_compressed_tools() {
    let compressed = vec![custom_tool(named(CONTENT_RETRIEVE_TOOL_NAME))];
    assert_eq!(
        merge_tools(&[], &compressed),
        vec![custom_tool(named(CONTENT_RETRIEVE_TOOL_NAME))]
    );
}

#[test]
fn merging_keeps_a_list_that_already_has_the_retrieval_tool() {
    let existing = vec![
        custom_tool(named(CONTENT_RETRIEVE_TOOL_NAME)),
        custom_tool(named("search")),
    ];
    let compressed = vec![custom_tool(named("another"))];
    assert_eq!(merge_tools(&existing, &compressed), existing);
}
