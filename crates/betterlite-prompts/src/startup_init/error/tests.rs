#![cfg(test)]

use std::error::Error as _;

use betterlite_models::prompt_spec::PromptSpec;
use rstest::rstest;

use crate::registry::mutation::prompt_registry_mutation::error::PromptRegistryError;
use crate::startup_init::error::InitPromptsError;

#[rstest]
#[case::prompt_body(
    r#"{"prompt_id":"greeting","litellm_params":"TOP-SECRET-PROMPT-BODY"}"#,
    "TOP-SECRET-PROMPT-BODY"
)]
#[case::api_secret(
    r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt"},"prompt_info":"sk-live-SECRET-API-KEY"}"#,
    "sk-live-SECRET-API-KEY"
)]
fn an_invalid_prompt_error_names_the_index_and_never_its_content(
    #[case] entry: &str,
    #[case] marker: &str,
) {
    let source = serde_json::from_str::<PromptSpec>(entry).expect_err("the entry is invalid");
    let error = InitPromptsError::InvalidPrompt { index: 2, source };
    let text = error.to_string();
    assert!(text.contains("index 2"), "{text}");
    assert!(!text.contains(marker), "{text}");
    let debug = format!("{error:?}");
    assert!(debug.contains("index 2"), "{debug}");
    assert!(!debug.contains(marker), "{debug}");
    assert!(error.source().is_some());
}

#[test]
fn an_initialize_error_prints_the_prompt_id_and_the_registry_cause() {
    let error = InitPromptsError::Initialize {
        prompt_id: "greeting".to_owned(),
        source: PromptRegistryError::UnsupportedIntegration {
            integration: "langfuse".to_owned(),
        },
    };
    assert_eq!(
        error.to_string(),
        "prompt greeting could not be initialized: Unsupported prompt: langfuse"
    );
    assert!(error.source().is_some());
}
