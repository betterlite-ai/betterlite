#![cfg(test)]

use crate::arguments::json::{Json, Object};
use crate::dotprompt_compilation::global::GlobalPromptDirectory;
use crate::dotprompt_compilation::init::manager::{DotpromptManager, PromptDataInput};

fn manager_with_greeting() -> DotpromptManager {
    let mut record = Object::new();
    record.insert("content".to_owned(), Json::Text("User: hi".to_owned()));
    let mut data = Object::new();
    data.insert("greeting".to_owned(), Json::Object(record));

    DotpromptManager::new(
        None,
        None,
        Some(PromptDataInput::Object(data)),
        None,
        &GlobalPromptDirectory::default(),
    )
    .expect("valid prompt data")
}

#[test]
fn returns_false_when_prompt_id_is_absent() {
    let manager = manager_with_greeting();
    assert!(!manager.should_run_prompt_management(None));
}

#[test]
fn returns_true_when_the_prompt_exists() {
    let manager = manager_with_greeting();
    assert!(manager.should_run_prompt_management(Some("greeting")));
}

#[test]
fn returns_false_when_the_prompt_does_not_exist() {
    let manager = manager_with_greeting();
    assert!(!manager.should_run_prompt_management(Some("missing")));
}

#[test]
fn returns_false_instead_of_propagating_when_the_lookup_itself_fails() {
    let manager = DotpromptManager::new(None, None, None, None, &GlobalPromptDirectory::default())
        .expect("no prompt_data to decode");

    assert!(!manager.should_run_prompt_management(Some("greeting")));
}
