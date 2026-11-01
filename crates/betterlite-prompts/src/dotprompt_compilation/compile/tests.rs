#![cfg(test)]

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::{AllMessageValues, PromptParams};
use indexmap::IndexMap;

use super::{CompileError, CompileFailure};
use crate::arguments::json::{Json, Object};
use crate::dotprompt_compilation::global::GlobalPromptDirectory;
use crate::dotprompt_compilation::init::manager::{DotpromptManager, PromptDataInput};

fn message(role: &str, content: &str) -> AllMessageValues {
    let mut fields = IndexMap::new();
    fields.insert("role".to_owned(), AllMessageValues::Text(role.to_owned()));
    fields.insert(
        "content".to_owned(),
        AllMessageValues::Text(content.to_owned()),
    );
    AllMessageValues::Object(fields)
}

fn manager_with_one_template(content: &str, metadata: Object) -> DotpromptManager {
    let mut record = Object::new();
    record.insert("content".to_owned(), Json::Text(content.to_owned()));
    record.insert("metadata".to_owned(), Json::Object(metadata));
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

fn manager_with_two_versions() -> DotpromptManager {
    let mut base_metadata = Object::new();
    base_metadata.insert("model".to_owned(), Json::Text("base-model".to_owned()));
    let mut base_record = Object::new();
    base_record.insert("content".to_owned(), Json::Text("User: base".to_owned()));
    base_record.insert("metadata".to_owned(), Json::Object(base_metadata));

    let mut versioned_metadata = Object::new();
    versioned_metadata.insert("model".to_owned(), Json::Text("v2-model".to_owned()));
    let mut versioned_record = Object::new();
    versioned_record.insert("content".to_owned(), Json::Text("User: v2".to_owned()));
    versioned_record.insert("metadata".to_owned(), Json::Object(versioned_metadata));

    let mut data = Object::new();
    data.insert("greeting".to_owned(), Json::Object(base_record));
    data.insert("greeting.v2".to_owned(), Json::Object(versioned_record));

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
fn a_requested_version_compiles_the_versioned_template_not_the_base() {
    let manager = manager_with_two_versions();

    let versioned = manager
        .compile_prompt_helper(Some("greeting"), None, Some(2))
        .expect("the v2 template exists");
    assert_eq!(versioned.prompt_template, vec![message("user", "v2")]);
    assert_eq!(versioned.prompt_template_model, Some("v2-model".to_owned()));

    let base = manager
        .compile_prompt_helper(Some("greeting"), None, None)
        .expect("the base template exists without a version");
    assert_eq!(base.prompt_template, vec![message("user", "base")]);
    assert_eq!(base.prompt_template_model, Some("base-model".to_owned()));
}

#[test]
fn sync_rejects_a_missing_prompt_id_with_the_exact_message() {
    let manager = manager_with_one_template("User: hi", Object::new());

    let error = manager
        .compile_prompt_helper(None, None, None)
        .expect_err("prompt_id is required");

    assert!(matches!(*error, CompileError::PromptIdRequired));
    assert_eq!(
        error.to_string(),
        "prompt_id is required for dotprompt manager"
    );
}

#[tokio::test]
async fn async_rejects_a_missing_prompt_id_with_the_exact_message() {
    let manager = manager_with_one_template("User: hi", Object::new());

    let error = manager
        .async_compile_prompt_helper(None, None, None)
        .await
        .expect_err("prompt_id is required");

    assert!(matches!(*error, CompileError::PromptIdRequired));
    assert_eq!(
        error.to_string(),
        "prompt_id is required for dotprompt manager"
    );
}

#[test]
fn successful_compile_returns_the_rendered_messages_model_and_params() {
    let mut metadata = Object::new();
    metadata.insert("model".to_owned(), Json::Text("gpt-4".to_owned()));
    metadata.insert(
        "temperature".to_owned(),
        Json::Number(serde_json::Number::from(1_i32)),
    );
    let manager = manager_with_one_template("User: hi", metadata);

    let client = manager
        .compile_prompt_helper(Some("greeting"), None, None)
        .expect("compiles");

    assert_eq!(client.prompt_id, Some("greeting".to_owned()));
    assert_eq!(client.prompt_template, vec![message("user", "hi")]);
    assert_eq!(client.prompt_template_model, Some("gpt-4".to_owned()));
    let mut expected_params = PromptParams::new();
    expected_params.insert(
        "temperature".to_owned(),
        ContractJson::Number(serde_json::Number::from(1_i32)),
    );
    assert_eq!(client.prompt_optional_params, Some(expected_params));
    assert_eq!(client.completed_messages, None);
}

#[test]
fn an_unknown_prompt_id_is_wrapped_with_the_exact_compile_error_message() {
    let manager = manager_with_one_template("User: hi", Object::new());

    let error = manager
        .compile_prompt_helper(Some("missing"), None, None)
        .expect_err("the prompt does not exist");

    assert!(matches!(
        *error,
        CompileError::Compile {
            source: CompileFailure::NotFound { .. },
            ..
        }
    ));
    assert_eq!(
        error.to_string(),
        "Error compiling prompt 'missing': Prompt 'missing' not found in prompt directory"
    );
}

#[test]
fn a_missing_prompt_with_a_nonzero_version_names_it_in_the_message() {
    let manager = manager_with_one_template("User: hi", Object::new());

    let error = manager
        .compile_prompt_helper(Some("missing"), None, Some(3))
        .expect_err("the prompt does not exist");

    assert_eq!(
        error.to_string(),
        "Error compiling prompt 'missing': Prompt 'missing' (version 3) not found in prompt directory"
    );
}

#[test]
fn a_missing_prompt_with_version_zero_omits_the_version_from_the_message() {
    let manager = manager_with_one_template("User: hi", Object::new());

    let error = manager
        .compile_prompt_helper(Some("missing"), None, Some(0))
        .expect_err("the prompt does not exist");

    assert_eq!(
        error.to_string(),
        "Error compiling prompt 'missing': Prompt 'missing' not found in prompt directory"
    );
}

#[test]
fn a_render_failure_is_wrapped_with_the_exact_compile_error_message() {
    let mut fields = Object::new();
    fields.insert("age".to_owned(), Json::Text("number".to_owned()));
    let mut input_schema = Object::new();
    input_schema.insert("schema".to_owned(), Json::Object(fields));
    let mut metadata = Object::new();
    metadata.insert("input".to_owned(), Json::Object(input_schema));
    let manager = manager_with_one_template("User: {{ age }}", metadata);

    let mut variables = Object::new();
    variables.insert("age".to_owned(), Json::Text("not a number".to_owned()));

    let error = manager
        .compile_prompt_helper(Some("greeting"), Some(&variables), None)
        .expect_err("age has the wrong type");

    assert!(matches!(
        *error,
        CompileError::Compile {
            source: CompileFailure::Render(_),
            ..
        }
    ));
    assert!(
        error
            .to_string()
            .starts_with("Error compiling prompt 'greeting': ")
    );
}

#[test]
fn a_missing_prompt_source_is_wrapped_with_the_exact_compile_error_message() {
    let manager = DotpromptManager::new(
        None,
        None,
        None,
        Some("greeting".to_owned()),
        &GlobalPromptDirectory::default(),
    )
    .expect("no prompt_data to decode");

    let error = manager
        .compile_prompt_helper(Some("greeting"), None, None)
        .expect_err("no prompt source was configured");

    assert!(matches!(
        *error,
        CompileError::Compile {
            source: CompileFailure::Manager(_),
            ..
        }
    ));
    assert!(
        error
            .to_string()
            .starts_with("Error compiling prompt 'greeting': ")
    );
}
