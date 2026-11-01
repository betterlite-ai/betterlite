#![cfg(test)]

use std::sync::{Arc, Mutex};

use rstest::rstest;

use super::{DotpromptManager, DotpromptManagerError, PromptDataInput};
use crate::arguments::json::{Json, Object};
use crate::dotprompt_compilation::global::GlobalPromptDirectory;

#[test]
fn integration_name_is_dotprompt() {
    assert_eq!(DotpromptManager::INTEGRATION_NAME, "dotprompt");
}

#[test]
fn prompt_directory_defaults_to_the_global_setting_when_not_given() {
    let global_directory = tempfile::tempdir().expect("creates a temp directory");
    let global = GlobalPromptDirectory(Arc::new(Mutex::new(Some(
        global_directory.path().display().to_string(),
    ))));

    let manager =
        DotpromptManager::new(None, None, None, None, &global).expect("no prompt_data to decode");

    manager
        .with_prompt_manager(|_| ())
        .expect("the global directory exists on disk, so it was used");
}

#[test]
fn an_explicit_prompt_directory_overrides_the_global_setting() {
    let explicit_directory = tempfile::tempdir().expect("creates a temp directory");
    let global = GlobalPromptDirectory(Arc::new(Mutex::new(Some(
        "/does/not/exist/at/all".to_owned(),
    ))));

    let manager = DotpromptManager::new(
        Some(explicit_directory.path().display().to_string()),
        None,
        None,
        None,
        &global,
    )
    .expect("no prompt_data to decode");

    manager
        .with_prompt_manager(|_| ())
        .expect("the explicit directory exists on disk, so the missing global one was never used");
}

#[test]
fn valid_json_string_prompt_data_is_decoded() {
    let manager = DotpromptManager::new(
        None,
        None,
        Some(PromptDataInput::Text(
            r#"{"greeting": {"content": "User: hi"}}"#.to_owned(),
        )),
        None,
        &GlobalPromptDirectory::default(),
    )
    .expect("valid JSON decodes");

    manager
        .with_prompt_manager(|_| ())
        .expect("prompt_data alone is a valid source");
}

#[test]
fn invalid_json_string_prompt_data_propagates_unwrapped() {
    let result = DotpromptManager::new(
        None,
        None,
        Some(PromptDataInput::Text("not json".to_owned())),
        None,
        &GlobalPromptDirectory::default(),
    );

    result.unwrap_err();
}

#[rstest]
#[case::nothing_set(None, None)]
#[case::empty_object(Some(Object::new()), None)]
fn lazy_prompt_manager_reports_missing_source_with_the_exact_message(
    #[case] prompt_data: Option<Object>,
    #[case] prompt_id: Option<String>,
) {
    let manager = DotpromptManager::new(
        None,
        None,
        prompt_data.map(PromptDataInput::Object),
        prompt_id,
        &GlobalPromptDirectory::default(),
    )
    .expect("no prompt_data to decode");

    let error = manager
        .with_prompt_manager(|_| ())
        .expect_err("no source was set");
    assert!(matches!(error, DotpromptManagerError::MissingPromptSource));
    assert_eq!(
        error.to_string(),
        "Either prompt_directory or prompt_data must be set before using dotprompt manager. Set litellm.global_prompt_directory, initialize with prompt_directory parameter, or provide prompt_data."
    );
}

#[test]
fn a_non_empty_prompt_data_object_is_a_valid_source() {
    let mut record = Object::new();
    record.insert("content".to_owned(), Json::Text("User: hi".to_owned()));
    let mut data = Object::new();
    data.insert("greeting".to_owned(), Json::Object(record));

    let manager = DotpromptManager::new(
        None,
        None,
        Some(PromptDataInput::Object(data)),
        None,
        &GlobalPromptDirectory::default(),
    )
    .expect("no prompt_data to decode");

    manager
        .with_prompt_manager(|_| ())
        .expect("prompt_data is a valid source");
}
