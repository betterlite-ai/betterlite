use betterlite_models::prompt_contracts::PromptIgnoreFlags;

use crate::orchestration::prompt_orchestration::{
    ChatCompletionPrompt, PromptManagementBackend as _,
};

use super::support::{
    FakeBackend, client_messages, compiled_client, ignoring_all, json, message, params,
    prompt_spec, request,
};

fn unchanged(model: &str) -> ChatCompletionPrompt {
    ChatCompletionPrompt {
        model: model.to_owned(),
        messages: client_messages(),
        non_default_params: params(r#"{"temperature":0.2,"max_tokens":10}"#),
    }
}

#[tokio::test]
async fn a_missing_prompt_id_returns_the_request_before_the_eligibility_check() {
    let backend = FakeBackend::ready(compiled_client());
    let dynamic = json("{}");

    let outcome = backend
        .get_chat_completion_prompt(request(None, "dotprompt/gpt-4", &dynamic))
        .await
        .expect("the request passes through");

    assert_eq!(outcome, unchanged("dotprompt/gpt-4"));
    assert!(backend.gate_calls().is_empty());
    assert!(backend.compile_calls().is_empty());
}

#[tokio::test]
async fn the_sync_hook_returns_the_request_unchanged_when_the_gate_is_false() {
    let backend = FakeBackend::ineligible(compiled_client());
    let dynamic = json("{}");

    let outcome = backend
        .get_chat_completion_prompt(request(Some("greeting"), "dotprompt/gpt-4", &dynamic))
        .await
        .expect("the request passes through");

    assert_eq!(outcome, unchanged("dotprompt/gpt-4"));
    assert_eq!(backend.gate_calls(), vec![Some("greeting".to_owned())]);
    assert!(backend.compile_calls().is_empty());
}

#[tokio::test]
async fn the_sync_hook_compiles_then_merges_params_and_uses_completed_messages() {
    let backend = FakeBackend::ready(compiled_client());
    let dynamic = json("{}");

    let outcome = backend
        .get_chat_completion_prompt(request(Some("greeting"), "dotprompt/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "template-model");
    assert_eq!(
        outcome.messages,
        vec![
            message("system", "template prompt"),
            message("user", "client question"),
        ]
    );
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.8"))
    );
    assert_eq!(
        outcome.non_default_params.get("max_tokens"),
        Some(&json("10"))
    );
}

#[tokio::test]
async fn the_sync_hook_honors_the_callers_ignore_flags() {
    let backend = FakeBackend::ready(compiled_client());
    let dynamic = json("{}");
    let mut completion = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    completion.ignore_flags = PromptIgnoreFlags {
        model: true,
        optional_params: true,
    };

    let outcome = backend
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "dotprompt/gpt-4");
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.2"))
    );
    assert_eq!(
        outcome.non_default_params.get("max_tokens"),
        Some(&json("10"))
    );
    assert_eq!(outcome.non_default_params.len(), 2);
}

#[tokio::test]
async fn the_sync_hook_resolves_the_spec_ignore_flags() {
    let backend = FakeBackend::ready(compiled_client());
    let dynamic = json("{}");
    let spec = prompt_spec(ignoring_all());
    let mut completion = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    completion.prompt_spec = Some(&spec);

    let outcome = backend
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "dotprompt/gpt-4");
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.2"))
    );
    assert_eq!(outcome.non_default_params.len(), 2);
}

#[tokio::test]
async fn the_sync_hook_compiles_without_the_prompt_spec() {
    let backend = FakeBackend::ready(compiled_client());
    let dynamic = json("{}");
    let spec = prompt_spec(PromptIgnoreFlags::default());
    let mut completion = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    completion.prompt_spec = Some(&spec);

    backend
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert!(backend.compile_calls()[0].spec.is_none());
}

#[tokio::test]
async fn the_async_hook_checks_the_gate_before_any_compilation() {
    let backend = FakeBackend::ineligible(compiled_client());
    let dynamic = json("{}");

    let outcome = backend
        .async_completion_prompt(request(None, "dotprompt/gpt-4", &dynamic))
        .await
        .expect("the request passes through");

    assert_eq!(outcome, unchanged("dotprompt/gpt-4"));
    assert_eq!(backend.gate_calls(), vec![None]);
    assert!(backend.compile_calls().is_empty());
}

#[tokio::test]
async fn the_async_hook_compiles_even_without_a_prompt_id() {
    let backend = FakeBackend::failing();
    let dynamic = json("{}");

    let error = backend
        .async_completion_prompt(request(None, "dotprompt/gpt-4", &dynamic))
        .await
        .expect_err("the helper fails");

    assert_eq!(
        error.to_string(),
        "Error compiling prompt: backend compile failed. Prompt id=None"
    );
    assert_eq!(backend.compile_calls()[0].id, None);
}

#[tokio::test]
async fn the_async_hook_passes_the_prompt_spec_to_the_helper() {
    let backend = FakeBackend::ready(compiled_client());
    let dynamic = json("{}");
    let spec = prompt_spec(PromptIgnoreFlags::default());
    let mut completion = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    completion.prompt_spec = Some(&spec);

    backend
        .async_completion_prompt(completion)
        .await
        .expect("completion prompt");

    let passed_id = backend.compile_calls()[0]
        .spec
        .as_ref()
        .map(|recorded| recorded.prompt_id.clone());
    assert_eq!(passed_id, Some("greeting".to_owned()));
}

#[tokio::test]
async fn the_async_hook_applies_the_same_post_processing() {
    let backend = FakeBackend::ready(compiled_client());
    let dynamic = json("{}");

    let outcome = backend
        .async_completion_prompt(request(Some("greeting"), "dotprompt/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "template-model");
    assert_eq!(
        outcome.messages,
        vec![
            message("system", "template prompt"),
            message("user", "client question"),
        ]
    );
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.8"))
    );
}
