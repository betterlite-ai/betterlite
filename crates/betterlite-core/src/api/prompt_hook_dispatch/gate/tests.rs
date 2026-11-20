#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::api::prompt_hook_dispatch::gate::should_run_prompt_hooks;
use crate::api::prompt_hook_dispatch::tests::support::{json, list, object, registry, store, text};

fn tools_with(id: Json) -> Vec<JsonObject> {
    vec![object(vec![
        ("type", text("file_search")),
        ("vector_store_ids", id),
    ])]
}

#[test]
fn a_prompt_id_runs_the_hooks() {
    assert!(should_run_prompt_hooks(
        &JsonObject::new(),
        Some("prompt-1"),
        None,
        None
    ));
}

#[test]
fn an_empty_prompt_id_does_not_run_the_hooks() {
    assert!(!should_run_prompt_hooks(
        &JsonObject::new(),
        Some(""),
        None,
        None
    ));
}

#[test]
fn an_agent_model_runs_the_hooks() {
    let params = object(vec![("model", text("litellm_agent/assistant"))]);
    assert!(should_run_prompt_hooks(&params, None, None, None));
}

#[test]
fn the_agent_prefix_is_checked_at_the_start_of_the_model() {
    let params = object(vec![("model", text("anthropic/litellm_agent/assistant"))]);
    assert!(!should_run_prompt_hooks(&params, None, None, None));
}

#[rstest]
#[case::number("42")]
#[case::null("null")]
#[case::flag("true")]
#[case::list(r#"["litellm_agent/assistant"]"#)]
#[case::object(r#"{"prefix":"litellm_agent/assistant"}"#)]
fn a_non_text_model_is_not_an_agent(#[case] value: &str) {
    let params = object(vec![("model", json(value))]);
    assert!(!should_run_prompt_hooks(&params, None, None, None));
}

#[rstest]
#[case::cache_control("cache_control_injection_points")]
#[case::knowledge_bases("knowledge_bases")]
#[case::vector_store_ids("vector_store_ids")]
fn each_dynamic_name_runs_the_hooks(#[case] name: &str) {
    let params = object(vec![(name, list(&["one"]))]);
    assert!(should_run_prompt_hooks(&params, None, None, None));
}

#[rstest]
#[case::cache_control("cache_control_injection_points")]
#[case::knowledge_bases("knowledge_bases")]
#[case::vector_store_ids("vector_store_ids")]
fn an_empty_dynamic_name_does_not_run_the_hooks(#[case] name: &str) {
    let params = object(vec![(name, Json::List(Vec::new()))]);
    assert!(!should_run_prompt_hooks(&params, None, None, None));
}

#[rstest]
#[case::empty_text(r#""""#)]
#[case::zero("0")]
#[case::flag("false")]
#[case::null("null")]
#[case::empty_object("{}")]
fn a_falsy_dynamic_name_does_not_run_the_hooks(#[case] value: &str) {
    let params = object(vec![("cache_control_injection_points", json(value))]);
    assert!(!should_run_prompt_hooks(&params, None, None, None));
}

#[test]
fn the_prompt_id_is_checked_before_the_vector_store_registry() {
    let registry = registry(vec![store("vs-1")]);
    let tools = tools_with(json("[1]"));
    assert!(should_run_prompt_hooks(
        &JsonObject::new(),
        Some("prompt-1"),
        Some(tools.as_slice()),
        Some(&registry)
    ));
}

#[test]
fn the_agent_model_is_checked_before_the_vector_store_registry() {
    let registry = registry(vec![store("vs-1")]);
    let tools = tools_with(json("[1]"));
    let params = object(vec![("model", text("litellm_agent/assistant"))]);
    assert!(should_run_prompt_hooks(
        &params,
        None,
        Some(tools.as_slice()),
        Some(&registry)
    ));
}

#[test]
fn a_dynamic_name_is_checked_before_the_vector_store_registry() {
    let registry = registry(vec![store("vs-1")]);
    let tools = tools_with(json("[1]"));
    let params = object(vec![("knowledge_bases", list(&["kb-1"]))]);
    assert!(should_run_prompt_hooks(
        &params,
        None,
        Some(tools.as_slice()),
        Some(&registry)
    ));
}

#[test]
fn a_stored_vector_store_in_the_tools_runs_the_hooks() {
    let registry = registry(vec![store("vs-1")]);
    let tools = tools_with(list(&["vs-1"]));
    assert!(should_run_prompt_hooks(
        &JsonObject::new(),
        None,
        Some(tools.as_slice()),
        Some(&registry)
    ));
}

#[test]
fn an_unknown_vector_store_does_not_run_the_hooks() {
    let registry = registry(vec![store("vs-1")]);
    let tools = tools_with(list(&["vs-2"]));
    assert!(!should_run_prompt_hooks(
        &JsonObject::new(),
        None,
        Some(tools.as_slice()),
        Some(&registry)
    ));
}

#[test]
fn without_a_registry_a_vector_store_tool_does_not_run_the_hooks() {
    let tools = tools_with(list(&["vs-1"]));
    assert!(!should_run_prompt_hooks(
        &JsonObject::new(),
        None,
        Some(tools.as_slice()),
        None
    ));
}

#[test]
fn a_malformed_vector_store_id_list_does_not_run_the_hooks() {
    let registry = registry(vec![store("vs-1")]);
    let tools = tools_with(json("[1]"));
    assert!(!should_run_prompt_hooks(
        &JsonObject::new(),
        None,
        Some(tools.as_slice()),
        Some(&registry)
    ));
}

#[test]
fn an_unhashable_vector_store_id_does_not_run_the_hooks() {
    let registry = registry(vec![store("vs-1")]);
    let tools = tools_with(json(r#"[{"vector_store_id":"vs-1"}]"#));
    assert!(!should_run_prompt_hooks(
        &JsonObject::new(),
        None,
        Some(tools.as_slice()),
        Some(&registry)
    ));
}
