use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::tools::openai_web_search_tool;
use crate::websearch_request_hooks::tests::{json, list, logger, object};
use crate::websearch_request_hooks::tool_rewrite::{encode, sync_forced_tool_choice};

fn converted_tools() -> Vec<Json> {
    vec![
        encode(&openai_web_search_tool()).unwrap(),
        Json::Object(object(
            r#"{"type":"function","function":{"name":"other_tool"}}"#,
        )),
    ]
}

#[test]
fn repoints_a_forced_choice_naming_the_converted_tool() {
    let choice = object(r#"{"type":"tool","name":"web_search"}"#);
    let synced = sync_forced_tool_choice(&Json::Object(choice), &converted_tools()).unwrap();
    assert_eq!(
        synced,
        Json::Object(object(r#"{"type":"tool","name":"litellm_web_search"}"#))
    );
}

#[test]
fn leaves_a_forced_choice_that_already_names_a_converted_tool() {
    let choice = Json::Object(object(r#"{"type":"tool","name":"litellm_web_search"}"#));
    let synced = sync_forced_tool_choice(&choice, &converted_tools()).unwrap();
    assert_eq!(synced, choice);
}

#[test]
fn leaves_a_forced_choice_naming_an_unchanged_function_tool() {
    let choice = Json::Object(object(r#"{"type":"tool","name":"other_tool"}"#));
    let synced = sync_forced_tool_choice(&choice, &converted_tools()).unwrap();
    assert_eq!(synced, choice);
}

#[test]
fn leaves_a_choice_naming_a_tool_with_a_function_field_of_another_type() {
    let choice = Json::Object(object(r#"{"type":"tool","name":"outer"}"#));
    let tools = vec![Json::Object(object(
        r#"{"type":"custom","name":"outer","function":{"name":"other_tool"}}"#,
    ))];
    let synced = sync_forced_tool_choice(&choice, &tools).unwrap();
    assert_eq!(synced, choice);
}

#[test]
fn preserves_the_other_tool_choice_fields() {
    let choice = object(r#"{"type":"tool","name":"web_search","disable_parallel_tool_use":true}"#);
    let synced = sync_forced_tool_choice(&Json::Object(choice), &converted_tools()).unwrap();
    assert_eq!(
        synced,
        Json::Object(object(
            r#"{"type":"tool","name":"litellm_web_search","disable_parallel_tool_use":true}"#
        ))
    );
}

#[rstest]
#[case::auto_string(r#""auto""#)]
#[case::auto_object(r#"{"type":"auto"}"#)]
#[case::any_object(r#"{"type":"any"}"#)]
#[case::null("null")]
fn leaves_non_forced_choices_untouched(#[case] source: &str) {
    let choice = json(source);
    let synced = sync_forced_tool_choice(&choice, &converted_tools()).unwrap();
    assert_eq!(synced, choice);
}

#[test]
fn the_pre_request_hook_syncs_the_forced_choice() {
    let request = object(
        r#"{"litellm_params":{"custom_llm_provider":"anthropic"},"tools":[{"type":"web_search_20250305","name":"web_search"}],"tool_choice":{"type":"tool","name":"web_search"}}"#,
    );
    let converted = logger(&["anthropic"])
        .async_pre_request_hook(&request)
        .unwrap()
        .unwrap();
    assert_eq!(
        converted.get("tool_choice"),
        Some(&Json::Object(object(
            r#"{"type":"tool","name":"litellm_web_search"}"#
        )))
    );
    let expected = encode(&crate::tools::get_litellm_web_search_tool()).unwrap();
    assert_eq!(list(&converted, "tools")[0], expected);
}
