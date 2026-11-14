use super::support;
use crate::langfuse_prompt_management::LangfusePromptManagementError;
use crate::langfuse_prompt_management::prompt::{optional_params_from_langfuse, template_model};
use crate::langfuse_rest::records::Prompt;

fn prompt(source: &str) -> Prompt {
    serde_json::from_str(source).expect("the prompt parses")
}

#[test]
fn the_model_key_is_filtered_and_the_rest_of_the_config_is_kept() {
    let parsed = prompt(&support::chat_prompt());
    let params = optional_params_from_langfuse(&parsed).expect("the optional params extract");
    assert_eq!(params.len(), 1);
    assert!(params.contains_key("temperature"));
    assert!(!params.contains_key("model"));
    assert_eq!(
        template_model(&parsed)
            .expect("the model extracts")
            .as_deref(),
        Some("gpt-4o")
    );
}

#[test]
fn an_empty_config_yields_no_params_and_no_model() {
    let parsed = prompt(
        r#"{"type":"text","prompt":"x","name":"n","version":1,"config":{},"labels":[],"tags":[]}"#,
    );
    assert!(
        optional_params_from_langfuse(&parsed)
            .expect("the optional params extract")
            .is_empty()
    );
    assert_eq!(template_model(&parsed).expect("the model extracts"), None);
}

#[test]
fn a_config_that_is_not_an_object_is_an_error() {
    let parsed = prompt(
        r#"{"type":"text","prompt":"x","name":"n","version":1,"config":"oops","labels":[],"tags":[]}"#,
    );
    assert!(matches!(
        optional_params_from_langfuse(&parsed),
        Err(LangfusePromptManagementError::ConfigType)
    ));
    assert!(matches!(
        template_model(&parsed),
        Err(LangfusePromptManagementError::ConfigType)
    ));
}

#[test]
fn a_model_that_is_not_a_string_is_an_error() {
    let parsed = prompt(
        r#"{"type":"text","prompt":"x","name":"n","version":1,"config":{"model":7},"labels":[],"tags":[]}"#,
    );
    assert!(matches!(
        template_model(&parsed),
        Err(LangfusePromptManagementError::ModelType)
    ));
}
