use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::deployment::ChunkHookContext;
use super::super::state::ResponsesStreamHookState;
use super::support::{
    ChunkAction, ChunkHook, completed_event, completed_with_id, event_json, hooks, json_object,
    redactor, text,
};
use crate::routes::CallType;

fn call_details() -> IndexMap<String, Json> {
    json_object(&[("model", text("gpt-4o"))])
}

fn context<'call>(
    explicit: Option<&'call str>,
    logging: Option<&'call str>,
    request_data: Option<&'call IndexMap<String, Json>>,
    model_call_details: &'call IndexMap<String, Json>,
) -> ChunkHookContext<'call> {
    ChunkHookContext {
        explicit_call_type: explicit,
        logging_call_type: logging,
        request_data,
        model_call_details,
    }
}

#[tokio::test]
async fn deployment_chunk_hooks() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let first = ChunkHook::new("first", ChunkAction::Keep);
    let replaced = completed_with_id("resp_2");
    let second = ChunkHook::new("second", ChunkAction::Replace(Box::new(replaced.clone())));
    hooks.chunk_hooks = vec![first.clone(), second.clone()];
    let mut callback = ResponsesStreamHookState::default();
    let details = call_details();
    let request_data = json_object(&[("model", text("request-model"))]);

    let returned = hooks
        .call_post_streaming_deployment_hook(
            &mut callback,
            context(
                Some("responses"),
                Some("acompletion"),
                Some(&request_data),
                &details,
            ),
            completed_event(),
        )
        .await;

    assert!(callback.post_streaming_hooks_ran);
    assert_eq!(first.calls(), vec!["first"]);
    assert_eq!(second.calls(), vec!["second"]);
    assert_eq!(first.call_types(), vec![Some(CallType::Responses)]);
    assert_eq!(second.call_types(), vec![Some(CallType::Responses)]);
    assert_eq!(
        first.requests()[0].get("model"),
        Some(&text("request-model"))
    );
    assert_eq!(
        second.requests()[0].get("model"),
        Some(&text("request-model"))
    );
    assert_eq!(event_json(&returned), event_json(&replaced));
}

#[tokio::test]
async fn deployment_chunk_hooks_fall_back_to_the_logging_call_type() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let hook = ChunkHook::new("hook", ChunkAction::Keep);
    hooks.chunk_hooks = vec![hook.clone()];
    let details = call_details();
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .call_post_streaming_deployment_hook(
            &mut callback,
            context(Some("bogus"), Some("acompletion"), None, &details),
            completed_event(),
        )
        .await;

    assert_eq!(hook.call_types(), vec![Some(CallType::Acompletion)]);

    let mut unknown = ResponsesStreamHookState::default();
    hooks
        .call_post_streaming_deployment_hook(
            &mut unknown,
            context(Some("bogus"), Some("also_bogus"), None, &details),
            completed_event(),
        )
        .await;

    assert_eq!(hook.call_types(), vec![Some(CallType::Acompletion), None]);
}

#[tokio::test]
async fn deployment_chunk_hooks_let_an_explicit_call_type_win() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let hook = ChunkHook::new("hook", ChunkAction::Keep);
    hooks.chunk_hooks = vec![hook.clone()];
    let details = call_details();
    let request_data = json_object(&[("model", text("request-model"))]);

    hooks
        .call_post_streaming_deployment_hook(
            &mut ResponsesStreamHookState::default(),
            context(
                Some("acompletion"),
                Some("responses"),
                Some(&request_data),
                &details,
            ),
            completed_event(),
        )
        .await;

    assert_eq!(hook.call_types(), vec![Some(CallType::Acompletion)]);
    assert_eq!(
        hook.requests()[0].get("model"),
        Some(&text("request-model"))
    );
}

#[tokio::test]
async fn deployment_chunk_hooks_fall_back_to_the_model_call_details() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let hook = ChunkHook::new("hook", ChunkAction::Keep);
    hooks.chunk_hooks = vec![hook.clone()];
    let details = call_details();
    let empty = IndexMap::new();

    hooks
        .call_post_streaming_deployment_hook(
            &mut ResponsesStreamHookState::default(),
            context(None, None, Some(&empty), &details),
            completed_event(),
        )
        .await;
    hooks
        .call_post_streaming_deployment_hook(
            &mut ResponsesStreamHookState::default(),
            context(None, None, None, &details),
            completed_event(),
        )
        .await;

    let requests = hook.requests();
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(request.get("model"), Some(&text("gpt-4o")));
    }
}

#[tokio::test]
async fn deployment_chunk_hooks_stop_at_a_failing_hook() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let replaced = completed_with_id("resp_2");
    let first = ChunkHook::new("first", ChunkAction::Replace(Box::new(replaced.clone())));
    let failing = ChunkHook::new("failing", ChunkAction::Fail);
    let later = ChunkHook::new(
        "later",
        ChunkAction::Replace(Box::new(completed_with_id("resp_3"))),
    );
    hooks.chunk_hooks = vec![first.clone(), failing.clone(), later.clone()];
    let details = call_details();
    let mut callback = ResponsesStreamHookState::default();

    let returned = hooks
        .call_post_streaming_deployment_hook(
            &mut callback,
            context(None, None, None, &details),
            completed_event(),
        )
        .await;

    assert_eq!(first.calls(), vec!["first"]);
    assert_eq!(failing.calls(), vec!["failing"]);
    assert!(later.calls().is_empty());
    assert!(!callback.post_streaming_hooks_ran);
    assert_eq!(event_json(&returned), event_json(&replaced));
}

#[tokio::test]
async fn deployment_chunk_hooks_keep_the_input_when_the_first_hook_fails() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    hooks.chunk_hooks = vec![ChunkHook::new("failing", ChunkAction::Fail)];
    let details = call_details();
    let mut callback = ResponsesStreamHookState::default();
    let input = completed_event();

    let returned = hooks
        .call_post_streaming_deployment_hook(
            &mut callback,
            context(None, None, None, &details),
            input.clone(),
        )
        .await;

    assert!(!callback.post_streaming_hooks_ran);
    assert_eq!(event_json(&returned), event_json(&input));
}

#[tokio::test]
async fn deployment_chunk_hooks_without_hooks_leave_the_chunk_alone() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let details = call_details();
    let mut callback = ResponsesStreamHookState::default();
    let input = completed_event();

    let returned = hooks
        .call_post_streaming_deployment_hook(
            &mut callback,
            context(None, None, None, &details),
            input.clone(),
        )
        .await;

    assert!(!callback.post_streaming_hooks_ran);
    assert_eq!(event_json(&returned), event_json(&input));
}
