#![cfg(test)]

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::PromptParams;

use crate::arguments::json::{Json, Object};
use crate::dotprompt_compilation::global::GlobalPromptDirectory;
use crate::dotprompt_compilation::init::manager::{DotpromptManager, PromptDataInput};

fn prompt_data(content: &str, metadata: Object) -> Object {
    let mut record = Object::new();
    record.insert("content".to_owned(), Json::Text(content.to_owned()));
    record.insert("metadata".to_owned(), Json::Object(metadata));
    let mut data = Object::new();
    data.insert("greeting".to_owned(), Json::Object(record));
    data
}

#[test]
fn optional_params_is_empty_when_metadata_has_no_extra_keys() {
    let manager = DotpromptManager::new(
        None,
        None,
        Some(PromptDataInput::Object(prompt_data(
            "User: hi",
            Object::new(),
        ))),
        None,
        &GlobalPromptDirectory::default(),
    )
    .expect("valid prompt data");

    let client = manager
        .compile_prompt_helper(Some("greeting"), None, None)
        .expect("compiles");

    assert_eq!(client.prompt_optional_params, Some(PromptParams::new()));
}

#[test]
fn optional_params_copies_non_restricted_metadata_keys() {
    let mut metadata = Object::new();
    metadata.insert(
        "temperature".to_owned(),
        Json::Number(serde_json::Number::from(1_i32)),
    );

    let manager = DotpromptManager::new(
        None,
        None,
        Some(PromptDataInput::Object(prompt_data("User: hi", metadata))),
        None,
        &GlobalPromptDirectory::default(),
    )
    .expect("valid prompt data");

    let client = manager
        .compile_prompt_helper(Some("greeting"), None, None)
        .expect("compiles");

    let mut expected = PromptParams::new();
    expected.insert(
        "temperature".to_owned(),
        ContractJson::Number(serde_json::Number::from(1_i32)),
    );
    assert_eq!(client.prompt_optional_params, Some(expected));
}
