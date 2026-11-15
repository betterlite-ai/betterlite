use std::sync::{Arc, Mutex};

use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, AgenticLoopRequestPatch, JsonMap};
use betterlite_models::decisions::Json;

use super::super::DEFAULT_MAX_TOKENS;
use super::super::executor::AnthropicContinuation;
use super::support::{
    ANTHROPIC_BODY, Fixture, NEXT_BODY, RecordingCallback, ScriptedExecutor, anthropic_fixture,
    callback_list, dispatch, fixture, plan_harness, plan_with_patch, recording, typed_kwargs,
};
use super::{assert_log, json, number, patched, stream_frames, text, without_model};
use crate::api::chat_agentic_loop::tests::support::{Script, capture, gate, json_map, overrides};

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
        text(&call.params, "_headroom_interception_x").as_deref(),
        Some("true")
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

pub(super) async fn converted_stream_wrapped() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let quiet = quiet_callback(&order);
    let mut converted = anthropic_fixture();
    converted.converted = true;
    let (log, _guard) = capture();
    let outcome = dispatch(
        &converted,
        &callback_list(&[&quiet]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("the converted stream is wrapped");
    assert_log(
        &log,
        "WebSearchInterception: Agentic loop completed, converting non-streaming response to fake stream",
    );
    let frames = stream_frames(outcome.expect("a wrapped stream"));
    let joined = String::from_utf8(frames.concat()).expect("frames are UTF-8");
    assert!(joined.starts_with("event: message_start\n"));
    assert!(joined.contains("event: message_stop\ndata: "));
    assert!(joined.contains("hello"));
}

pub(super) async fn unconverted_answers_keep_response() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let quiet = quiet_callback(&order);
    let mut converted = anthropic_fixture();
    converted.converted = false;
    let plain = dispatch(
        &converted,
        &callback_list(&[&quiet]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("an unconverted anthropic answer keeps the response");
    assert!(plain.is_none());
    let mut responses_converted = fixture();
    responses_converted.converted = true;
    let native = dispatch(
        &responses_converted,
        &callback_list(&[&quiet]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("the responses surface is never wrapped");
    assert!(native.is_none());
}
