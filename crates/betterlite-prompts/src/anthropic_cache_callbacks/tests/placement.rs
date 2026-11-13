use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::PromptParams;

use crate::anthropic_cache_callbacks::error::CacheControlCallbackError;
use crate::orchestration::prompt_orchestration::{
    PromptManagementBackend as _, PromptOrchestrationError,
};

use super::support::{
    Facts, chat_request, dynamic_params, hook_with, messages, params, plain_hook,
};

const ANTHROPIC_POINT: &str = r#"[{"location": "message", "role": "user", "index": null, "control": null, "_litellm_openai_dialect": false}]"#;
const UNSTAMPED_POINT: &str =
    r#"[{"location": "message", "role": "user", "index": null, "control": null}]"#;

fn marked_params(points: &str) -> PromptParams {
    params(&format!(
        r#"{{"cache_control_injection_points": {points}}}"#
    ))
}

fn openai_params(points: &str) -> PromptParams {
    params(&format!(
        r#"{{"prompt_cache_options": {{"mode": "explicit"}}, "cache_control_injection_points": {points}}}"#
    ))
}

fn user_message() -> Vec<ContractJson> {
    messages(r#"[{"role": "user", "content": "hi"}]"#)
}

#[tokio::test]
async fn the_sync_hook_places_cache_control_at_the_configured_point() {
    let dynamic = dynamic_params();
    let placed = plain_hook()
        .get_chat_completion_prompt(chat_request(
            "claude-sonnet-4-5",
            user_message(),
            marked_params(ANTHROPIC_POINT),
            &dynamic,
        ))
        .await
        .expect("the placement succeeds");
    assert_eq!(placed.model, "claude-sonnet-4-5");
    assert_eq!(
        placed.messages,
        messages(r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#)
    );
    assert!(placed.non_default_params.is_empty());
}

#[tokio::test]
async fn the_async_hook_returns_the_placement_the_sync_hook_returns() {
    let dynamic = dynamic_params();
    let hook = plain_hook();
    let input = user_message();
    let sync = hook
        .get_chat_completion_prompt(chat_request(
            "claude-sonnet-4-5",
            input.clone(),
            marked_params(ANTHROPIC_POINT),
            &dynamic,
        ))
        .await
        .expect("the placement succeeds");
    let asynchronous = hook
        .async_completion_prompt(chat_request(
            "claude-sonnet-4-5",
            input.clone(),
            marked_params(ANTHROPIC_POINT),
            &dynamic,
        ))
        .await
        .expect("the delegation succeeds");
    assert_eq!(asynchronous, sync);
    // the base prompt manager would hand the messages back untouched, since this hook never runs as one
    assert_ne!(asynchronous.messages, input);
}

#[tokio::test]
async fn the_hook_leaves_a_request_without_injection_points_unchanged() {
    let dynamic = dynamic_params();
    let placed = plain_hook()
        .async_completion_prompt(chat_request(
            "claude-sonnet-4-5",
            user_message(),
            params(r#"{"temperature": 0.5}"#),
            &dynamic,
        ))
        .await
        .expect("the delegation succeeds");
    assert_eq!(placed.messages, user_message());
    assert_eq!(placed.non_default_params, params(r#"{"temperature": 0.5}"#));
}

#[tokio::test]
async fn the_hook_reads_the_injected_model_facts_for_the_dialect() {
    let dynamic = dynamic_params();
    let hook = hook_with(Facts::supporting_breakpoints("gpt-5.6", "openai"));
    let placed = hook
        .get_chat_completion_prompt(chat_request(
            "gpt-5.6",
            user_message(),
            openai_params(UNSTAMPED_POINT),
            &dynamic,
        ))
        .await
        .expect("the placement succeeds");
    assert_eq!(
        placed.messages,
        messages(
            r#"[{"role": "user", "content": [{"type": "text", "text": "hi", "prompt_cache_breakpoint": {"mode": "explicit"}}]}]"#
        )
    );
    assert_eq!(
        placed.non_default_params,
        params(r#"{"prompt_cache_options": {"mode": "explicit"}}"#)
    );
}

#[tokio::test]
async fn the_hook_reads_the_injected_api_base_for_the_dialect() {
    let dynamic = dynamic_params();
    let mut hook = hook_with(Facts::supporting_breakpoints("gpt-5.6", "openai"));
    hook.global_api_base = Some("https://gateway.example.test/v1".to_owned());
    let placed = hook
        .async_completion_prompt(chat_request(
            "gpt-5.6",
            user_message(),
            marked_params(UNSTAMPED_POINT),
            &dynamic,
        ))
        .await
        .expect("the delegation succeeds");
    assert_eq!(
        placed.messages,
        messages(r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#)
    );
}

#[tokio::test]
async fn the_hook_reads_the_injected_environment_for_the_dialect() {
    let dynamic = dynamic_params();
    let mut hook = hook_with(Facts::supporting_breakpoints("gpt-5.6", "openai"));
    hook.environment.insert(
        "OPENAI_BASE_URL".to_owned(),
        "https://gateway.example.test/v1".to_owned(),
    );
    let placed = hook
        .async_completion_prompt(chat_request(
            "gpt-5.6",
            user_message(),
            marked_params(UNSTAMPED_POINT),
            &dynamic,
        ))
        .await
        .expect("the delegation succeeds");
    assert_eq!(
        placed.messages,
        messages(r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#)
    );
}

#[tokio::test]
async fn a_malformed_injection_point_list_is_a_placement_error() {
    let dynamic = dynamic_params();
    let error = plain_hook()
        .get_chat_completion_prompt(chat_request(
            "claude-sonnet-4-5",
            user_message(),
            params(r#"{"cache_control_injection_points": "default"}"#),
            &dynamic,
        ))
        .await
        .expect_err("a string is not a point list");
    assert!(matches!(
        error,
        PromptOrchestrationError::Compile {
            source: CacheControlCallbackError::Placement { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn messages_that_do_not_match_the_prompt_wire_format_are_rejected() {
    let dynamic = dynamic_params();
    let error = plain_hook()
        .get_chat_completion_prompt(chat_request(
            "claude-sonnet-4-5",
            messages("[42]"),
            marked_params(ANTHROPIC_POINT),
            &dynamic,
        ))
        .await
        .expect_err("a number is not a message");
    assert!(matches!(
        error,
        PromptOrchestrationError::Compile {
            source: CacheControlCallbackError::Wire { .. },
            ..
        }
    ));
}
