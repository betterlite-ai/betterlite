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
fn prompt_file_to_json_splits_frontmatter_from_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("greeting.prompt");
    fs::write(&path, "---\nmodel: gpt-4\n---\nHello {{ name }}!").unwrap();

    let result = PromptManager::prompt_file_to_json(&path).unwrap();
    assert_eq!(result["content"].as_str(), Some("Hello {{ name }}!"));
    assert_eq!(
        result["metadata"].as_object().unwrap()["model"].as_str(),
        Some("gpt-4")
    );
}

#[rstest]
fn prompt_file_to_json_handles_a_file_without_frontmatter() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plain.prompt");
    fs::write(&path, "Just plain content").unwrap();

    let result = PromptManager::prompt_file_to_json(&path).unwrap();
    assert_eq!(result["content"].as_str(), Some("Just plain content"));
    assert!(result["metadata"].as_object().unwrap().is_empty());
}

#[rstest]
fn a_missing_file_surfaces_the_read_error_with_its_cause() {
    let path = Path::new("/nonexistent/betterlite-prompts-test-file.prompt");
    let error = PromptManager::prompt_file_to_json(path).unwrap_err();
    let message = error.to_string();
    assert!(message.starts_with(&format!(
        "failed to read prompt file '{}': ",
        path.display()
    )));
    assert!(message.contains("No such file"));
}

#[rstest]
fn get_all_prompts_as_json_maps_every_stored_prompt() {
    let mut manager = PromptManager::new(None, None, None, None).unwrap();
    manager
        .add_prompt(
            "greeting",
            "hi".to_owned(),
            Some(object(r#"{"model": "gpt-4"}"#)),
        )
        .unwrap();
    manager
        .add_prompt("farewell", "bye".to_owned(), None)
        .unwrap();

    let result = manager.get_all_prompts_as_json();
    assert_eq!(
        result["greeting"].as_object().unwrap()["content"].as_str(),
        Some("hi")
    );
    assert_eq!(
        result["greeting"].as_object().unwrap()["metadata"]
            .as_object()
            .unwrap()["model"]
            .as_str(),
        Some("gpt-4")
    );
    assert_eq!(
        result["farewell"].as_object().unwrap()["content"].as_str(),
        Some("bye")
    );
}
