#![cfg(test)]

use super::super::PromptManager;
use crate::arguments::json::Object;
use rstest::rstest;
use std::fs;
use std::path::Path;

fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

#[rstest]
fn loads_every_dot_prompt_file_in_a_directory_nonrecursively() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("greeting.prompt"),
        "---\nmodel: gpt-4\n---\nHi {{ name }}",
    )
    .unwrap();
    fs::write(directory.path().join("other.txt"), "not a prompt").unwrap();
    fs::create_dir_all(directory.path().join("sub")).unwrap();
    fs::write(directory.path().join("sub/nested.prompt"), "ignored").unwrap();

    let manager = PromptManager::new(None, Some(directory.path()), None, None).unwrap();
    let template = manager.get_prompt("greeting", None).unwrap();
    assert_eq!(template.content, "Hi {{ name }}");
    assert_eq!(template.model, Some("gpt-4".to_owned()));
    assert!(manager.get_prompt("nested", None).is_none());
}

#[rstest]
fn a_malformed_prompt_file_is_skipped_without_failing_the_load() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("bad.prompt"),
        "---\nmodel: [unterminated\n---\nbody",
    )
    .unwrap();
    fs::write(directory.path().join("good.prompt"), "fine").unwrap();

    let manager = PromptManager::new(None, Some(directory.path()), None, None).unwrap();
    assert!(manager.get_prompt("bad", None).is_none());
    assert_eq!(manager.get_prompt("good", None).unwrap().content, "fine");
}

#[rstest]
fn missing_directory_raises_with_the_exact_message() {
    let path = Path::new("/nonexistent/betterlite-prompts-test-dir");
    let error = PromptManager::new(None, Some(path), None, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Prompt directory does not exist: {}", path.display())
    );
}

#[rstest]
fn prompt_file_without_prompt_id_raises_the_exact_message() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("solo.prompt");
    fs::write(&file, "hi").unwrap();

    let error = PromptManager::new(None, None, None, Some(&file)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "prompt_id is required when prompt_file is provided"
    );
}

#[rstest]
fn prompt_file_is_registered_under_the_supplied_prompt_id() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("solo.prompt");
    fs::write(&file, "---\nmodel: gpt-4\n---\nHello").unwrap();

    let manager = PromptManager::new(Some("solo"), None, None, Some(&file)).unwrap();
    assert_eq!(manager.get_prompt("solo", None).unwrap().content, "Hello");
}

#[rstest]
fn flat_prompt_data_with_prompt_id_registers_under_that_id() {
    let prompt_data = object(r#"{"content": "hi {{ name }}", "metadata": {"model": "gpt-4"}}"#);
    let manager = PromptManager::new(Some("x"), None, Some(&prompt_data), None).unwrap();
    let template = manager.get_prompt("x", None).unwrap();
    assert_eq!(template.content, "hi {{ name }}");
    assert_eq!(template.model, Some("gpt-4".to_owned()));
}

#[rstest]
fn keyed_prompt_data_registers_every_entry_under_its_own_key() {
    let prompt_data = object(
        r#"{
            "a": {"content": "content a", "metadata": {}},
            "b": {"content": "content b", "metadata": {}}
        }"#,
    );
    let manager = PromptManager::new(None, None, Some(&prompt_data), None).unwrap();
    assert_eq!(manager.get_prompt("a", None).unwrap().content, "content a");
    assert_eq!(manager.get_prompt("b", None).unwrap().content, "content b");
}

#[rstest]
fn a_malformed_json_record_is_skipped_without_failing_the_others() {
    let prompt_data = object(
        r#"{
            "a": {"content": "ok", "metadata": {}},
            "b": {"content": "ignored", "metadata": {"model": 4}}
        }"#,
    );
    let manager = PromptManager::new(None, None, Some(&prompt_data), None).unwrap();
    assert_eq!(manager.get_prompt("a", None).unwrap().content, "ok");
    assert!(manager.get_prompt("b", None).is_none());
}
