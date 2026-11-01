#![cfg(test)]

use super::super::PromptManager;
use crate::arguments::json::Object;
use rstest::rstest;
use std::fs;

fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

#[rstest]
fn reload_prompts_clears_and_reloads_from_the_directory() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("greeting.prompt"), "hi").unwrap();

    let mut manager = PromptManager::new(None, Some(directory.path()), None, None).unwrap();
    assert!(manager.get_prompt("greeting", None).is_some());

    fs::remove_file(directory.path().join("greeting.prompt")).unwrap();
    fs::write(directory.path().join("farewell.prompt"), "bye").unwrap();

    manager.reload_prompts().unwrap();
    assert!(manager.get_prompt("greeting", None).is_none());
    assert_eq!(manager.get_prompt("farewell", None).unwrap().content, "bye");
}

#[rstest]
fn reload_prompts_without_a_directory_only_clears_existing_prompts() {
    let prompt_data = object(r#"{"content": "hi", "metadata": {}}"#);
    let mut manager = PromptManager::new(Some("x"), None, Some(&prompt_data), None).unwrap();
    assert!(manager.get_prompt("x", None).is_some());

    manager.reload_prompts().unwrap();
    assert!(manager.get_prompt("x", None).is_none());
}

#[rstest]
fn add_prompt_stores_a_template_built_from_the_supplied_fields() {
    let mut manager = PromptManager::new(None, None, None, None).unwrap();
    manager
        .add_prompt(
            "greeting",
            "Hi {{ name }}".to_owned(),
            Some(object(r#"{"model": "gpt-4"}"#)),
        )
        .unwrap();

    let template = manager.get_prompt("greeting", None).unwrap();
    assert_eq!(template.content, "Hi {{ name }}");
    assert_eq!(template.model, Some("gpt-4".to_owned()));
    assert_eq!(template.template_id, Some("greeting".to_owned()));
}

#[rstest]
fn add_prompt_rejects_malformed_metadata() {
    let mut manager = PromptManager::new(None, None, None, None).unwrap();
    let result = manager.add_prompt("greeting", "hi".to_owned(), Some(object(r#"{"model": 4}"#)));
    assert!(result.is_err());
}

#[rstest]
fn load_prompts_from_json_data_merges_into_the_existing_store() {
    let mut manager = PromptManager::new(None, None, None, None).unwrap();
    manager
        .add_prompt("existing", "e".to_owned(), None)
        .unwrap();

    let prompt_data = object(r#"{"added": {"content": "a", "metadata": {}}}"#);
    manager.load_prompts_from_json_data(&prompt_data);

    assert_eq!(manager.get_prompt("existing", None).unwrap().content, "e");
    assert_eq!(manager.get_prompt("added", None).unwrap().content, "a");
}
