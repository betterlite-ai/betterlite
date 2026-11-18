use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::failure::await_pending_logging;
use super::super::state::ResponsesStreamHookState;
use super::support::{
    END, Metadata, START, SuccessHook, base_state, block_on_local, completed_event, event_json,
    hooks, json_object, json_value, number, path, redactor, text, timestamp,
};

fn payload_details() -> IndexMap<String, Json> {
    json_object(&[
        ("response_cost", number("0.25")),
        ("model", text("details-model")),
        (
            "litellm_params",
            json_value(&[("model", text("param-model"))]),
        ),
    ])
}

#[tokio::test]
async fn post_success_metadata() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let metadata = Metadata::recording();
    let hook = SuccessHook::recording();
    hooks.metadata = Some(std::sync::Arc::<Metadata>::clone(&metadata));
    hooks.success_hook = Some(std::sync::Arc::<SuccessHook>::clone(&hook));
    let mut state = base_state();
    state.request_data = json_object(&[
        ("model", text("request-model")),
        ("user_api_key", text("key-1")),
    ]);
    state.completed_response = Some(completed_event());
    let mut callback = ResponsesStreamHookState::default();
    let details = payload_details();

    hooks
        .run_post_success_hooks(&mut state, &mut callback, &details, timestamp(END))
        .await;
    await_pending_logging(&mut callback).await;

    let calls = metadata.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.model, "gpt-4o");
    assert!(!call.include_overhead);
    assert_eq!(call.start_time, timestamp(START));
    assert_eq!(call.end_time, timestamp(END));
    assert_eq!(
        path(&call.result, &["response", "id"]),
        Some(&text("resp_1"))
    );
    assert_eq!(call.kwargs.get("user_api_key"), Some(&text("key-1")));
    assert_eq!(call.kwargs.get("response_cost"), Some(&number("0.25")));
    assert_eq!(call.kwargs.get("model"), Some(&text("details-model")));
    assert_eq!(
        path(
            call.kwargs
                .get("litellm_params")
                .expect("the params are set"),
            &["model"]
        ),
        Some(&text("param-model"))
    );

    let hook_calls = hook.calls();
    assert_eq!(hook_calls.len(), 1);
    assert_eq!(hook_calls[0].call_type, "responses");
    assert_eq!(hook_calls[0].request_data, call.kwargs);
    let completed = state.completed_response.as_ref().expect("the event is set");
    assert_eq!(hook_calls[0].response, event_json(completed));
}

#[rstest::rstest]
#[case::absent(None)]
#[case::invalid(Some("bogus"))]
#[tokio::test]
async fn post_success_metadata_defaults_the_call_type_and_params(#[case] explicit: Option<&str>) {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let hook = SuccessHook::recording();
    hooks.success_hook = Some(std::sync::Arc::<SuccessHook>::clone(&hook));
    let mut state = base_state();
    state.call_type = explicit.map(str::to_owned);
    state.request_data = json_object(&[("model", text("request-model"))]);
    state.completed_response = Some(completed_event());
    let mut callback = ResponsesStreamHookState::default();
    let details = json_object(&[("response_cost", number("0.25"))]);

    hooks
        .run_post_success_hooks(&mut state, &mut callback, &details, timestamp(END))
        .await;
    await_pending_logging(&mut callback).await;

    let calls = hook.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].call_type, "responses");
    assert_eq!(
        calls[0].request_data.get("litellm_params"),
        Some(&json_value(&[]))
    );
}

#[tokio::test]
async fn post_success_metadata_keeps_the_explicit_call_type_and_request_params() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let hook = SuccessHook::recording();
    hooks.success_hook = Some(std::sync::Arc::<SuccessHook>::clone(&hook));
    let mut state = base_state();
    state.call_type = Some("acompletion".to_owned());
    state.request_data = json_object(&[(
        "litellm_params",
        json_value(&[("model", text("request-model"))]),
    )]);
    state.completed_response = Some(completed_event());
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .run_post_success_hooks(&mut state, &mut callback, &IndexMap::new(), timestamp(END))
        .await;
    await_pending_logging(&mut callback).await;

    let calls = hook.calls();
    assert_eq!(calls[0].call_type, "acompletion");
    assert_eq!(
        path(
            calls[0]
                .request_data
                .get("litellm_params")
                .expect("the request params stay"),
            &["model"]
        ),
        Some(&text("request-model"))
    );
}

#[tokio::test]
async fn post_success_metadata_swallows_a_metadata_failure() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    hooks.metadata = Some(Metadata::failing());
    let hook = SuccessHook::recording();
    hooks.success_hook = Some(std::sync::Arc::<SuccessHook>::clone(&hook));
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .run_post_success_hooks(&mut state, &mut callback, &IndexMap::new(), timestamp(END))
        .await;
    await_pending_logging(&mut callback).await;

    assert_eq!(hook.calls().len(), 1);
}

#[tokio::test]
async fn post_success_metadata_swallows_a_hook_failure() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let metadata = Metadata::recording();
    hooks.metadata = Some(std::sync::Arc::<Metadata>::clone(&metadata));
    hooks.success_hook = Some(SuccessHook::failing());
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .run_post_success_hooks(&mut state, &mut callback, &IndexMap::new(), timestamp(END))
        .await;
    await_pending_logging(&mut callback).await;

    assert_eq!(metadata.calls().len(), 1);
    assert!(callback.pending_logging_tasks.is_empty());
}

#[tokio::test]
async fn post_success_metadata_without_a_completed_response_does_nothing() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let metadata = Metadata::recording();
    let hook = SuccessHook::recording();
    hooks.metadata = Some(std::sync::Arc::<Metadata>::clone(&metadata));
    hooks.success_hook = Some(std::sync::Arc::<SuccessHook>::clone(&hook));
    let mut state = base_state();
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .run_post_success_hooks(&mut state, &mut callback, &IndexMap::new(), timestamp(END))
        .await;

    assert!(metadata.calls().is_empty());
    assert!(hook.calls().is_empty());
    assert!(callback.pending_logging_tasks.is_empty());
}

#[test]
fn post_success_metadata_runs_inline_without_a_loop() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let hook = SuccessHook::recording();
    hooks.success_hook = Some(std::sync::Arc::<SuccessHook>::clone(&hook));
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    let mut callback = ResponsesStreamHookState::default();
    let details = IndexMap::new();
    let end = timestamp(END);

    block_on_local(async {
        hooks
            .run_post_success_hooks(&mut state, &mut callback, &details, end)
            .await;
    });

    assert!(callback.pending_logging_tasks.is_empty());
    assert_eq!(hook.calls().len(), 1);
    assert_eq!(hook.calls()[0].call_type, "responses");
}

struct MutatingMetadata;

impl super::super::ports::ResponseMetadataTarget for MutatingMetadata {
    fn update_response_metadata(
        &self,
        call: super::super::ports::ResponseMetadataCall<'_>,
    ) -> Result<(), super::super::ports::PortFailure> {
        let response = super::super::events::nested_response_mut(call.result).unwrap();
        response
            .extra
            .insert("metadata_ready".to_owned(), Json::Bool(true));
        Ok(())
    }
}

#[tokio::test]
async fn metadata_mutation_precedes_success_hook() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let hook = SuccessHook::recording();
    hooks.success_hook = Some(std::sync::Arc::<SuccessHook>::clone(&hook));
    hooks.metadata = Some(std::sync::Arc::new(MutatingMetadata));
    let mut state = base_state();
    state.completed_response = Some(completed_event());
    let mut callback = ResponsesStreamHookState::default();
    hooks
        .run_post_success_hooks(&mut state, &mut callback, &IndexMap::new(), timestamp(END))
        .await;
    await_pending_logging(&mut callback).await;
    assert_eq!(
        path(&hook.calls()[0].response, &["response", "metadata_ready"]),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        path(
            &event_json(state.completed_response.as_ref().unwrap()),
            &["response", "metadata_ready"]
        ),
        Some(&Json::Bool(true))
    );
}
