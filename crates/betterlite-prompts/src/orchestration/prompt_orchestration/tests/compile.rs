use crate::orchestration::prompt_orchestration::{
    AsyncCompilePromptCall, CompilePromptCall, PromptManagementBackend as _,
    PromptOrchestrationError,
};

use super::support::{FakeBackend, client_messages, compiled_client, json, message};

#[tokio::test]
async fn compile_prompt_calls_the_helper_and_appends_the_client_messages_after_the_template() {
    let backend = FakeBackend::ready(compiled_client());
    let dynamic = json("{}");

    let compiled = backend
        .compile_prompt(CompilePromptCall {
            prompt_id: "greeting",
            client_messages: client_messages(),
            prompt_variables: None,
            dynamic_callback_params: &dynamic,
            prompt_spec: None,
            prompt_label: Some("prod"),
            prompt_version: Some(2),
        })
        .await
        .expect("compiled prompt");

    assert_eq!(compiled.prompt_id.as_deref(), Some("greeting"));
    assert_eq!(
        compiled.prompt_template,
        vec![message("system", "template prompt")]
    );
    assert_eq!(
        compiled.completed_messages,
        Some(vec![
            message("system", "template prompt"),
            message("user", "client question"),
        ])
    );
    let calls = backend.compile_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id.as_deref(), Some("greeting"));
    assert_eq!(calls[0].label.as_deref(), Some("prod"));
    assert_eq!(calls[0].version, Some(2));
}

#[tokio::test]
async fn async_compile_prompt_populates_completed_messages_the_same_way() {
    let backend = FakeBackend::ready(compiled_client());
    let dynamic = json("{}");

    let compiled = backend
        .async_compile_prompt(AsyncCompilePromptCall {
            prompt_id: Some("greeting"),
            client_messages: client_messages(),
            prompt_variables: None,
            dynamic_callback_params: &dynamic,
            prompt_spec: None,
            prompt_label: None,
            prompt_version: None,
        })
        .await
        .expect("compiled prompt");

    assert_eq!(
        compiled.completed_messages,
        Some(vec![
            message("system", "template prompt"),
            message("user", "client question"),
        ])
    );
    assert!(backend.compile_calls()[0].id.is_some());
}

#[tokio::test]
async fn a_helper_failure_is_wrapped_with_the_prompt_id() {
    let backend = FakeBackend::failing();
    let dynamic = json("{}");

    let error = backend
        .compile_prompt(CompilePromptCall {
            prompt_id: "greeting",
            client_messages: client_messages(),
            prompt_variables: None,
            dynamic_callback_params: &dynamic,
            prompt_spec: None,
            prompt_label: None,
            prompt_version: None,
        })
        .await
        .expect_err("the helper fails");

    assert!(matches!(error, PromptOrchestrationError::Compile { .. }));
    assert_eq!(
        error.to_string(),
        "Error compiling prompt: backend compile failed. Prompt id=greeting"
    );
}

#[tokio::test]
async fn a_helper_failure_without_a_prompt_id_still_names_the_call() {
    let backend = FakeBackend::failing();
    let dynamic = json("{}");

    let error = backend
        .async_compile_prompt(AsyncCompilePromptCall {
            prompt_id: None,
            client_messages: client_messages(),
            prompt_variables: None,
            dynamic_callback_params: &dynamic,
            prompt_spec: None,
            prompt_label: None,
            prompt_version: None,
        })
        .await
        .expect_err("the helper fails");

    assert_eq!(
        error.to_string(),
        "Error compiling prompt: backend compile failed. Prompt id=None"
    );
}
