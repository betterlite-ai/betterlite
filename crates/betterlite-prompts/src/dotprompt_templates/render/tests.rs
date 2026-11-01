#![cfg(test)]

use super::super::PromptManager;
use crate::arguments::json::Object;
use rstest::rstest;

fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

fn manager_with(prompt_id: &str, content: &str, metadata: &str) -> PromptManager {
    let mut manager = PromptManager::new(None, None, None, None).unwrap();
    manager
        .add_prompt(prompt_id, content.to_owned(), object(metadata))
        .unwrap();
    manager
}

#[rstest]
fn renders_a_template_with_the_supplied_variables() {
    let manager = manager_with("greeting", "Hello {{ name }}!", "{}");
    let variables = object(r#"{"name": "world"}"#);
    assert_eq!(
        manager.render("greeting", &variables, None).unwrap(),
        "Hello world!"
    );
}

#[rstest]
fn validates_input_against_the_schema_before_rendering() {
    let manager = manager_with(
        "greeting",
        "Hello {{ name }}!",
        r#"{"input": {"schema": {"name": "string"}}}"#,
    );
    let variables = object(r#"{"name": 5}"#);
    manager.render("greeting", &variables, None).unwrap_err();
}

#[rstest]
fn a_render_error_wraps_the_cause() {
    let manager = manager_with("greeting", "{{ undefined_helper() }}", "{}");
    manager
        .render("greeting", &Object::new(), None)
        .unwrap_err();
}

#[rstest]
fn not_found_without_a_version_omits_the_version_suffix() {
    let manager = PromptManager::new(None, None, None, None).unwrap();
    let error = manager.render("missing", &Object::new(), None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Prompt 'missing' not found. Available prompts: []"
    );
}

#[rstest]
fn not_found_with_a_nonzero_version_includes_the_version_suffix() {
    let manager = PromptManager::new(None, None, None, None).unwrap();
    let error = manager
        .render("missing", &Object::new(), Some(2))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Prompt 'missing' (version 2) not found. Available prompts: []"
    );
}

#[rstest]
fn not_found_with_version_zero_omits_the_suffix_like_python_truthiness() {
    let manager = PromptManager::new(None, None, None, None).unwrap();
    let error = manager
        .render("missing", &Object::new(), Some(0))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Prompt 'missing' not found. Available prompts: []"
    );
}

#[rstest]
fn render_escapes_html_special_characters_with_markupsafes_exact_table() {
    let manager = manager_with("greeting", "Hello {{ name }}!", "{}");
    let variables = object(r#"{"name": "O'Brien & <Sons>"}"#);
    assert_eq!(
        manager.render("greeting", &variables, None).unwrap(),
        "Hello O&#39;Brien &amp; &lt;Sons&gt;!"
    );
}

#[rstest]
fn render_rejects_a_template_past_the_byte_limit() {
    let content = "x".repeat(0x0001_0001);
    let manager = manager_with("huge", &content, "{}");
    manager.render("huge", &Object::new(), None).unwrap_err();
}

#[rstest]
fn render_rejects_an_output_past_the_byte_limit() {
    let manager = manager_with("echo", "{{ name }}", "{}");
    let mut variables = Object::new();
    variables.insert(
        "name".to_owned(),
        crate::arguments::json::Json::Text("x".repeat(0x0010_0001)),
    );
    manager.render("echo", &variables, None).unwrap_err();
}

#[rstest]
fn not_found_lists_the_available_prompt_ids() {
    let manager = manager_with("greeting", "hi", "{}");
    let error = manager.render("missing", &Object::new(), None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Prompt 'missing' not found. Available prompts: ['greeting']"
    );
}
