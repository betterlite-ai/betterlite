use std::sync::{Arc, Mutex};

use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, AgenticLoopRequestPatch, JsonMap};
use betterlite_models::decisions::Json;

use super::super::AgenticResult;
use super::super::DEFAULT_MAX_TOKENS;
use super::super::executor::AnthropicContinuation;
use super::support::{
    ANTHROPIC_BODY, Fixture, NEXT_BODY, REFUSED_BODY, RecordingCallback, ScriptedExecutor,
    anthropic_fixture, callback_list, dispatch, fixture, gated_script, legacy_script, plan_harness,
    plan_script, plan_with_patch, recording, refused_tools, typed_kwargs,
};
use super::{assert_log, json, number, patched, text, without_model, wrapped_frames};
use crate::api::chat_agentic_loop::tests::support::{
    Script, capture, gate, json_map, overrides, plan, raw,
};

async fn anthropic_call(fixture: Fixture, plan: &AgenticLoopPlan) -> AnthropicContinuation {
    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    plan_harness(fixture, Script::default())
        .anthropic(plan, &executor)
        .await
        .expect("the anthropic follow-up runs");
    let calls = executor.anthropic_calls();
    calls.into_iter().next().expect("the follow-up was sent")
}

pub(super) async fn model_ladder() {
    let mut logged = anthropic_fixture();
    logged.agentic_loop_params = Some(json_map(&[("model", "\"logged-model\"")]));
    let patched_plan = plan_with_patch(patched());
    let patched_call = anthropic_call(logged.clone(), &patched_plan).await;
    assert_eq!(patched_call.model, "patched-model");
    assert_eq!(patched_call.messages.len(), 1);
    assert_eq!(
        patched_call.messages[0].0.get(),
        r#"{"role":"user","content":"continue"}"#
    );
    let unpatched = plan_with_patch(without_model(&patched()));
    let logged_call = anthropic_call(logged, &unpatched).await;
    assert_eq!(logged_call.model, "logged-model");
    let request_call = anthropic_call(anthropic_fixture(), &unpatched).await;
    assert_eq!(request_call.model, "claude-3-5-sonnet");
}

pub(super) async fn merged_kwargs_max_tokens_wins() {
    let mut ladder = anthropic_fixture();
    ladder.optional_params = json_map(&[("max_tokens", "500")]);
    ladder.kwargs = typed_kwargs(&[("max_tokens", "900")]);
    let patch = AgenticLoopRequestPatch {
        max_tokens: Some(700),
        ..without_model(&patched())
    };
    let call = anthropic_call(ladder, &plan_with_patch(patch)).await;
    assert_eq!(call.max_tokens, 900);
}

pub(super) async fn patch_beats_optional_then_optional_used() {
    let mut ladder = anthropic_fixture();
    ladder.optional_params = json_map(&[("max_tokens", "500")]);
    let patch = AgenticLoopRequestPatch {
        max_tokens: Some(700),
        ..without_model(&patched())
    };
    let patched_call = anthropic_call(ladder.clone(), &plan_with_patch(patch)).await;
    assert_eq!(patched_call.max_tokens, 700);
    let optional = plan_with_patch(without_model(&patched()));
    let optional_call = anthropic_call(ladder, &optional).await;
    assert_eq!(optional_call.max_tokens, 500);
    assert!(optional_call.params.get("max_tokens").is_none());
}

pub(super) async fn kwargs_then_default_max_tokens() {
    let mut only_kwargs = anthropic_fixture();
    only_kwargs.kwargs = typed_kwargs(&[("max_tokens", "900")]);
    let unpatched = plan_with_patch(without_model(&patched()));
    let kwargs_call = anthropic_call(only_kwargs, &unpatched).await;
    assert_eq!(kwargs_call.max_tokens, 900);
    let default_call = anthropic_call(anthropic_fixture(), &unpatched).await;
    assert_eq!(default_call.max_tokens, DEFAULT_MAX_TOKENS);
}

pub(super) async fn merged_mapping() {
    let mut merged = anthropic_fixture();
    merged.optional_params = json_map(&[("request_only", "\"one\""), ("top_k", "\"five\"")]);
    merged.kwargs = typed_kwargs(&[
        ("_compression_interception_x", "true"),
        ("_headroom_interception_x", "true"),
        ("_websearch_interception_x", "true"),
        ("kept", "\"yes\""),
        ("litellm_logging_obj", "{}"),
        ("request_only", "\"nine\""),
        ("stream", "true"),
        ("top_k", "\"nine\""),
    ]);
    let mut harness = plan_harness(merged, Script::default());
    harness.state.settings.fingerprints = vec![Json::Text("old".to_owned())];
    let overlay = AgenticLoopRequestPatch {
        kwargs: json_map(&[
            ("_agentic_loop_depth", "\"99\""),
            ("_agentic_loop_fingerprints", "[]"),
            ("max_agentic_loops", "\"99\""),
            ("patched_only", "\"two\""),
            ("top_k", "\"seven\""),
        ]),
        ..without_model(&patched())
    };
    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    harness
        .anthropic(&plan_with_patch(overlay), &executor)
        .await
        .expect("the merged mapping is captured");
    let calls = executor.anthropic_calls();
    assert_merged_mapping(&calls[0]);
}

fn assert_merged_mapping(call: &AnthropicContinuation) {
    assert!(call.stream);
    assert_eq!(text(&call.params, "top_k").as_deref(), Some("seven"));
    assert_eq!(text(&call.params, "request_only").as_deref(), Some("one"));
    assert_eq!(text(&call.params, "patched_only").as_deref(), Some("two"));
    assert_eq!(text(&call.params, "kept").as_deref(), Some("yes"));
    assert_eq!(
        call.params.get("litellm_logging_obj"),
        None,
        "litellm_logging_obj survived"
    );
    assert_eq!(
        call.params.get("_websearch_interception_x"),
        None,
        "_websearch_interception_x survived"
    );
    assert_eq!(
        call.params.get("_compression_interception_x"),
        None,
        "_compression_interception_x survived"
    );
    assert_eq!(
        call.params.get("_headroom_interception_x"),
        Some(&json("true")),
        "_headroom_interception_x survived"
    );
    assert_eq!(number(&call.params, "_agentic_loop_depth"), Some(1));
    assert_eq!(number(&call.params, "max_agentic_loops"), Some(3));
    assert_eq!(
        call.params.get("_agentic_loop_fingerprints").cloned(),
        Some(json("[\"old\",\"fingerprint-1\"]"))
    );
}

fn quiet_callback(order: &Arc<Mutex<Vec<String>>>) -> Arc<RecordingCallback> {
    recording(
        "w",
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(false, JsonMap::new())),
        order,
    )
}

async fn converted_dispatch(
    fixture: &Fixture,
    callbacks: &[&Arc<RecordingCallback>],
    executor: &ScriptedExecutor,
) -> String {
    let outcome = dispatch(fixture, &callback_list(callbacks), executor)
        .await
        .expect("the converted answer is wrapped");
    let joined = wrapped_frames(outcome.expect("a wrapped converted stream"));
    assert!(joined.starts_with("event: message_start\n"));
    joined
}

pub(super) async fn converted_wrap_sites() {
    let (log, _guard) = capture();
    let order = Arc::new(Mutex::new(Vec::new()));
    let quiet = quiet_callback(&order);
    let mut streamed = anthropic_fixture();
    streamed.converted = true;
    let stream_frames = converted_dispatch(
        &streamed,
        &[&quiet],
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await;
    assert!(stream_frames.contains("event: message_stop\ndata: "));
    assert!(stream_frames.contains("hello"));
    let legacy = recording("l", legacy_script(ANTHROPIC_BODY), &order);
    let legacy_frames = converted_dispatch(
        &converted_fixture(),
        &[&legacy],
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await;
    assert!(legacy_frames.contains("hello"));
    let tools = refused_tools("web_search", "call-ws");
    let overridden = recording(
        "o",
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, tools.clone()))
            .with_plan(plan(None, Some(raw(ANTHROPIC_BODY)), false, false)),
        &order,
    );
    let override_frames = converted_dispatch(
        &converted_fixture(),
        &[&overridden],
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await;
    assert!(override_frames.contains("hello"));
    let mut terminate = plan(None, None, true, false);
    terminate.stop_reason = Some("done".to_owned());
    let terminated = recording(
        "t",
        Script::default()
            .with_overrides(overrides(true, true, true))
            .with_gate(gate(true, tools.clone()))
            .with_plan(terminate)
            .with_post(raw(ANTHROPIC_BODY)),
        &order,
    );
    let terminate_frames = converted_dispatch(
        &converted_fixture(),
        &[&terminated],
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await;
    assert!(terminate_frames.contains("hello"));
    let mut refused = converted_fixture();
    refused.response = raw(REFUSED_BODY);
    refused.kwargs = typed_kwargs(&[("_agentic_loop_depth", "1"), ("max_agentic_loops", "1")]);
    let gated = recording("s", gated_script(&tools), &order);
    let refusal_frames = converted_dispatch(
        &refused,
        &[&gated],
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await;
    assert!(refusal_frames.contains("after"));
    assert!(refusal_frames.contains("end_turn"));
    let planned = recording("x", plan_script(&tools, patched()), &order);
    let execution_frames = converted_dispatch(
        &converted_fixture(),
        &[&planned],
        &ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY),
    )
    .await;
    assert!(execution_frames.contains("hello"));
    assert_log(
        &log,
        "WebSearchInterception: Agentic loop completed, converting non-streaming response to fake stream",
    );
    assert_log(&log, "Agentic loop terminated by callback=t reason=done");
    assert_log(&log, "LiteLLM.AgenticLoopRefused: ending turn");
}

fn converted_fixture() -> Fixture {
    let mut converted = anthropic_fixture();
    converted.converted = true;
    converted
}

pub(super) async fn anthropic_post_failure_keeps_answer() {
    let plan = plan_with_patch(patched());
    let harness = plan_harness(
        anthropic_fixture(),
        Script::default().with_post_error("post broke"),
    );
    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    let (log, _guard) = capture();
    let response = harness
        .anthropic(&plan, &executor)
        .await
        .expect("a post failure keeps the provider answer");
    assert_eq!(response.get(), ANTHROPIC_BODY);
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in async_post_agentic_loop_response_hook [call_id=call-1 model=claude-3-5-sonnet]: post broke",
    );
    let records = harness.callback.records();
    assert_eq!(records.post.len(), 1);
    assert!(records.cleanup.is_empty());
}

async fn dispatch_without_rerun(fixture: Fixture) -> Option<AgenticResult> {
    let order = Arc::new(Mutex::new(Vec::new()));
    let quiet = quiet_callback(&order);
    dispatch(
        &fixture,
        &callback_list(&[&quiet]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("the dispatcher runs")
}

pub(super) async fn unconverted_answers_keep_response() {
    let mut converted = anthropic_fixture();
    converted.converted = false;
    let plain = dispatch_without_rerun(converted).await;
    assert!(plain.is_none());
    let mut responses_converted = fixture();
    responses_converted.converted = true;
    let native = dispatch_without_rerun(responses_converted).await;
    assert!(native.is_none());
}

pub(super) async fn absent_logging_and_non_objects_keep_response() {
    let mut unlogged = anthropic_fixture();
    unlogged.converted = true;
    unlogged.call_id = None;
    let unlogged_outcome = dispatch_without_rerun(unlogged).await;
    assert!(unlogged_outcome.is_none());
    let mut listed = anthropic_fixture();
    listed.converted = true;
    listed.response = raw("[1,2]");
    let listed_outcome = dispatch_without_rerun(listed).await;
    assert!(listed_outcome.is_none());
}
