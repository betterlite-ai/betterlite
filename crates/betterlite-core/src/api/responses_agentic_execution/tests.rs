#![cfg(test)]

mod anthropic;
mod callbacks;
mod followup;
mod patch_failures;
pub(crate) mod support;
mod termination;

use std::str::from_utf8;
use std::sync::{Arc, Mutex};

use betterlite_context::agentic_loop_plan::{AgenticLoopRequestPatch, JsonMap};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::AgenticResult;
use crate::api::chat_agentic_loop::tests::support::{Capture, json_map};
use crate::api::responses_agentic_execution::error::AgenticExecutionError;
use support::{
    Fixture, RecordingCallback, continue_messages, fixture, gated_script, legacy_script,
    plan_script, recording, refused_tools, typed_kwargs,
};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a test JSON document")
}

fn response_text(result: AgenticResult) -> String {
    match result {
        AgenticResult::Response(raw) => raw.get().to_owned(),
        AgenticResult::Stream(_) => panic!("expected a response result"),
    }
}

fn stream_frames(result: AgenticResult) -> Vec<Vec<u8>> {
    match result {
        AgenticResult::Stream(stream) => stream.collect(),
        AgenticResult::Response(_) => panic!("expected a stream result"),
    }
}

fn wrapped_frames(result: AgenticResult) -> String {
    let frames = stream_frames(result);
    from_utf8(&frames.concat())
        .expect("frames are UTF-8")
        .to_owned()
}

fn dispatch_error(
    outcome: Result<Option<AgenticResult>, AgenticExecutionError>,
    message: &str,
) -> AgenticExecutionError {
    match outcome {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

fn assert_log(log: &Capture, needle: &str) {
    let lines = log.messages();
    assert!(
        lines.iter().any(|line| line.contains(needle)),
        "missing {needle:?} in {lines:?}"
    );
}

fn number(map: &IndexMap<String, Json>, key: &str) -> Option<i64> {
    let Some(Json::Number(value)) = map.get(key) else {
        return None;
    };
    value.as_i64()
}

fn text(map: &IndexMap<String, Json>, key: &str) -> Option<String> {
    let Some(Json::Text(value)) = map.get(key) else {
        return None;
    };
    Some(value.clone())
}

fn raw_text(map: &JsonMap, key: &str) -> Option<String> {
    map.get(key).map(|value| value.get().to_owned())
}

fn gated(
    name: &'static str,
    order: &Arc<Mutex<Vec<String>>>,
    tools: &JsonMap,
) -> Arc<RecordingCallback> {
    recording(name, gated_script(tools), order)
}

fn legacy(
    name: &'static str,
    order: &Arc<Mutex<Vec<String>>>,
    body: &str,
) -> Arc<RecordingCallback> {
    recording(name, legacy_script(body), order)
}

fn plan_callback(
    name: &'static str,
    order: &Arc<Mutex<Vec<String>>>,
    tools: &JsonMap,
    request_patch: AgenticLoopRequestPatch,
) -> Arc<RecordingCallback> {
    recording(name, plan_script(tools, request_patch), order)
}

fn followup_fixture() -> (Fixture, JsonMap) {
    let mut fixture = fixture();
    fixture.optional_params = json_map(&[
        ("temperature", "0.2"),
        ("stream", "true"),
        ("_code_interpreter_interception_converted_stream", "true"),
    ]);
    fixture.kwargs = typed_kwargs(&[
        ("_agentic_loop_depth", "0"),
        ("_agentic_loop_fingerprints", "[\"old\"]"),
        ("_code_interpreter_interception_converted_stream", "true"),
        ("_code_interpreter_interception_item_parts", "3"),
        ("_compression_interception_item_parts", "1"),
        ("_headroom_interception_state", "2"),
        ("_websearch_interception_converted_stream", "true"),
        ("api_base", "\"http://provider.invalid/v1\""),
        ("input", "[]"),
        ("litellm_logging_obj", "{}"),
        ("max_agentic_loops", "2"),
        ("model", "\"gpt-4o\""),
        ("unrelated", "\"kept\""),
    ]);
    (fixture, refused_tools("web_search", "call-ws"))
}

fn followup_patch() -> AgenticLoopRequestPatch {
    let mut request = patched();
    request.model = Some("gpt-4o-mini".to_owned());
    request.tools = Some(Vec::new());
    request.optional_params = json_map(&[("top_p", "0.9")]);
    request.kwargs = json_map(&[("extra_kw", "\"kept\""), ("temperature", "0.5")]);
    request
}

fn patched() -> AgenticLoopRequestPatch {
    AgenticLoopRequestPatch {
        model: Some("patched-model".to_owned()),
        messages: Some(continue_messages()),
        tools: None,
        max_tokens: None,
        optional_params: JsonMap::new(),
        kwargs: JsonMap::new(),
    }
}

fn without_model(request_patch: &AgenticLoopRequestPatch) -> AgenticLoopRequestPatch {
    AgenticLoopRequestPatch {
        model: None,
        ..request_patch.clone()
    }
}

#[tokio::test]
async fn agentic_callback_order() {
    callbacks::plan_hooks_in_order().await;
    callbacks::legacy_answers().await;
    callbacks::null_legacy_is_no_result().await;
    callbacks::plan_error_moves_on().await;
    callbacks::run_loop_false_moves_on().await;
    callbacks::no_callbacks_is_no_result().await;
    callbacks::optional_tools_reach_the_gate().await;
    callbacks::invalid_optional_tools_raise().await;
}

#[tokio::test]
async fn responses_followup_http() {
    followup::http_followup().await;
    followup::captured_followup().await;
    followup::failed_send_is_logged().await;
    followup::anthropic_surface_is_refused().await;
}

#[tokio::test]
async fn agentic_override_termination_and_safety() {
    termination::override_answers().await;
    termination::terminate_posts().await;
    termination::non_object_finalize_keeps_value();
    termination::post_failure_caught().await;
    termination::max_loops_refuses_responses().await;
    termination::repeated_fingerprint_refuses().await;
    termination::anthropic_refusal_finalizes().await;
    termination::streaming_refusal_raises().await;
    termination::client_tool_use_kept().await;
}

#[tokio::test]
async fn continuation_patch_failures() {
    patch_failures::missing_input_raises().await;
    patch_failures::provider_failure_cleans_up().await;
    patch_failures::post_failure_keeps_answer().await;
    patch_failures::cleanup_failures_swallowed().await;
    patch_failures::dispatcher_plan_failure_moves_on().await;
    patch_failures::anthropic_plan_failure_logged().await;
}

#[tokio::test]
async fn anthropic_continuation_and_wrapping() {
    anthropic::model_ladder().await;
    anthropic::merged_kwargs_max_tokens_wins().await;
    anthropic::patch_beats_optional_then_optional_used().await;
    anthropic::kwargs_then_default_max_tokens().await;
    anthropic::merged_mapping().await;
    anthropic::converted_wrap_sites().await;
    anthropic::unconverted_answers_keep_response().await;
    anthropic::absent_logging_and_non_objects_keep_response().await;
    anthropic::anthropic_post_failure_keeps_answer().await;
}
