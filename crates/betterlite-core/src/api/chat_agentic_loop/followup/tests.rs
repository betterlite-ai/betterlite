#![cfg(test)]

use std::sync::Arc;

use rstest::rstest;

use super::execute;
use crate::api::chat_agentic_loop::error::ChatCallError;
use crate::api::chat_agentic_loop::tests::support::{
    FakeContinuation, FollowupParts, MODEL_RESPONSE, Script, ScriptedCallback, StopError,
    call_error, capture, empty_stream, first_call, followup_parts, json_map, messages, metadata_of,
    overrides, patch, plan, raw, raw_text, response_text, tool_messages,
};
use crate::api::chat_agentic_loop::{ChatLoopResult, LoggingHandle};

const HOOKED_RESPONSE: &str = r#"{"id":"chatcmpl-hooked","object":"chat.completion","created":1,"model":"gpt-4o","choices":[]}"#;

async fn run(parts: &FollowupParts) -> Result<ChatLoopResult, ChatCallError<StopError>> {
    execute(&parts.followup()).await
}

#[rstest]
#[case::bare_model(None, "gpt-4o", "openai", "openai/gpt-4o")]
#[case::patched_model(Some("claude-3"), "gpt-4o", "openai", "openai/claude-3")]
#[case::patched_qualified(Some("anthropic/claude-3"), "gpt-4o", "openai", "anthropic/claude-3")]
#[case::qualified_original(None, "vertex_ai/gemini", "vertex_ai", "vertex_ai/gemini")]
#[tokio::test]
async fn the_followup_model_gets_the_provider_prefix(
    #[case] patched: Option<&'static str>,
    #[case] original: &'static str,
    #[case] provider: &'static str,
    #[case] expected: &'static str,
) {
    let mut parts = followup_parts();
    parts.model = original;
    parts.provider = provider;
    if let Some(model) = patched {
        parts.plan.request_patch.as_mut().unwrap().model = Some(model.to_owned());
    }
    run(&parts).await.unwrap();
    assert_eq!(parts.continuation.calls()[0].model, expected);
}

#[tokio::test]
async fn the_params_are_the_original_with_the_patch_laid_over() {
    let mut parts = followup_parts();
    parts.optional_params = json_map(&[("temperature", "0.2"), ("top_p", "0.9")]);
    let mut patch = patch(Some(messages()));
    patch.optional_params = json_map(&[("temperature", "0.5"), ("max_tokens", "10")]);
    patch.tools = Some(tool_messages());
    parts.plan = plan(Some(patch), None, false, true);
    run(&parts).await.unwrap();
    let call = first_call(&parts.continuation);
    assert_eq!(
        raw_text(&call.params, "temperature").as_deref(),
        Some("0.5")
    );
    assert_eq!(raw_text(&call.params, "top_p").as_deref(), Some("0.9"));
    assert_eq!(raw_text(&call.params, "max_tokens").as_deref(), Some("10"));
    assert_eq!(
        call.params.get("tools").map(|tools| tools.get()),
        Some(r#"[{"type":"function","function":{"name":"get_weather"}}]"#)
    );
    assert_eq!(call.params.len(), 4);
    assert_eq!(call.messages.len(), 1);
}

#[tokio::test]
async fn the_tool_choice_is_dropped_unless_the_patch_sets_it() {
    let mut parts = followup_parts();
    parts.optional_params = json_map(&[("tool_choice", "\"auto\"")]);
    run(&parts).await.unwrap();
    assert!(
        parts.continuation.calls()[0]
            .params
            .get("tool_choice")
            .is_none()
    );
}

#[tokio::test]
async fn the_patch_tool_choice_replaces_the_original() {
    let mut parts = followup_parts();
    parts.optional_params = json_map(&[("tool_choice", "\"auto\"")]);
    let mut patch = patch(Some(messages()));
    patch.optional_params = json_map(&[("tool_choice", "\"none\"")]);
    parts.plan = plan(Some(patch), None, false, true);
    run(&parts).await.unwrap();
    let call = first_call(&parts.continuation);
    assert_eq!(
        raw_text(&call.params, "tool_choice").as_deref(),
        Some("\"none\"")
    );
}

#[tokio::test]
async fn the_followup_kwargs_drop_the_plumbing_and_other_interceptors() {
    let mut parts = followup_parts();
    parts.kwargs = json_map(&[
        ("acompletion", "true"),
        ("litellm_logging_obj", "{}"),
        ("custom_llm_provider", "\"openai\""),
        ("model_alias_map", "{}"),
        ("stream_response", "{}"),
        ("custom_prompt_dict", "{}"),
        ("_agentic_loop_api_surface", "\"chat_completions\""),
        ("_websearch_interception_active", "true"),
        ("_compression_interception_x", "1"),
        ("_headroom_interception_y", "1"),
        ("_code_interpreter_interception_active", "true"),
        ("user", "\"u-1\""),
        ("litellm_metadata", r#"{"trace":"t-1"}"#),
    ]);
    run(&parts).await.unwrap();
    let call = first_call(&parts.continuation);
    let mut keys: Vec<&str> = call.kwargs.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "_agentic_loop_depth",
            "_agentic_loop_fingerprints",
            "_code_interpreter_interception_active",
            "litellm_metadata",
            "max_agentic_loops",
            "user",
        ]
    );
    assert_eq!(raw_text(&call.kwargs, "user").as_deref(), Some("\"u-1\""));
}

#[tokio::test]
async fn the_metadata_merges_the_old_entries_and_the_loop_markers() {
    let mut parts = followup_parts();
    parts.kwargs = json_map(&[
        ("_code_interpreter_interception_active", "true"),
        ("litellm_metadata", r#"{"trace":"t-1"}"#),
    ]);
    run(&parts).await.unwrap();
    let call = first_call(&parts.continuation);
    let metadata = metadata_of(&call.kwargs);
    assert_eq!(raw_text(&metadata, "trace").as_deref(), Some("\"t-1\""));
    assert_eq!(
        raw_text(&metadata, "_code_interpreter_interception_active").as_deref(),
        Some("true")
    );
    assert_eq!(
        raw_text(&metadata, "_agentic_loop_depth").as_deref(),
        Some("1")
    );
    assert_eq!(
        raw_text(&metadata, "max_agentic_loops").as_deref(),
        Some("3")
    );
    assert_eq!(
        raw_text(&metadata, "_agentic_loop_fingerprints").as_deref(),
        Some(r#"["fingerprint-1"]"#)
    );
    assert!(metadata.get("_websearch_interception_active").is_none());
    assert!(metadata.get("_agentic_loop_api_surface").is_none());
}

#[tokio::test]
async fn a_non_object_metadata_is_replaced_by_the_loop_markers() {
    let mut parts = followup_parts();
    parts.kwargs = json_map(&[("litellm_metadata", "[1,2]")]);
    run(&parts).await.unwrap();
    let call = first_call(&parts.continuation);
    let metadata = metadata_of(&call.kwargs);
    assert_eq!(metadata.len(), 3);
    assert!(metadata.get("litellm_metadata").is_none());
}

#[tokio::test]
async fn the_patch_kwargs_win_over_the_request_kwargs() {
    let mut parts = followup_parts();
    parts.kwargs = json_map(&[("user", "\"request\"")]);
    let mut patch = patch(Some(messages()));
    patch.kwargs = json_map(&[("user", "\"patch\"")]);
    parts.plan = plan(Some(patch), None, false, true);
    run(&parts).await.unwrap();
    let call = first_call(&parts.continuation);
    assert_eq!(raw_text(&call.kwargs, "user").as_deref(), Some("\"patch\""));
}

#[tokio::test]
async fn the_params_are_not_repeated_as_kwargs() {
    let mut parts = followup_parts();
    parts.optional_params = json_map(&[("temperature", "0.2")]);
    parts.kwargs = json_map(&[
        ("temperature", "0.9"),
        ("model", "\"other\""),
        ("messages", "[]"),
        ("user", "\"u\""),
    ]);
    run(&parts).await.unwrap();
    let call = first_call(&parts.continuation);
    assert!(call.kwargs.get("temperature").is_none());
    assert!(call.kwargs.get("model").is_none());
    assert!(call.kwargs.get("messages").is_none());
    assert_eq!(raw_text(&call.kwargs, "user").as_deref(), Some("\"u\""));
}

#[tokio::test]
async fn the_injected_counters_follow_the_settings_and_the_fingerprint() {
    let mut parts = followup_parts();
    parts.settings.depth = 2;
    parts.settings.max_loops = 5;
    parts.settings.fingerprints = vec!["old".to_owned()];
    run(&parts).await.unwrap();
    let call = first_call(&parts.continuation);
    assert_eq!(
        raw_text(&call.kwargs, "_agentic_loop_depth").as_deref(),
        Some("3")
    );
    assert_eq!(
        raw_text(&call.kwargs, "max_agentic_loops").as_deref(),
        Some("5")
    );
    assert_eq!(
        raw_text(&call.kwargs, "_agentic_loop_fingerprints").as_deref(),
        Some(r#"["old","fingerprint-1"]"#)
    );
}

#[tokio::test]
async fn the_post_hook_returns_the_hooked_response() {
    let mut parts = followup_parts();
    parts.plan.stop_reason = Some("done".to_owned());
    parts.callback = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(false, false, true))
            .with_post(raw(r#"{"hooked":true}"#)),
    ));
    let result = run(&parts).await.unwrap();
    assert_eq!(response_text(result), r#"{"hooked":true}"#);
    let records = parts.callback.records();
    assert_eq!(records.post_calls.len(), 1);
    assert_eq!(records.post_calls[0].stop_reason.as_deref(), Some("done"));
}

#[tokio::test]
async fn a_post_hook_error_keeps_the_followup_response_and_logs_the_call_id() {
    let mut parts = followup_parts();
    parts.callback = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(false, false, true))
            .with_post_error("post boom"),
    ));
    let (logs, _guard) = capture();
    let result = run(&parts).await.unwrap();
    assert_eq!(response_text(result), MODEL_RESPONSE);
    assert_eq!(
        logs.messages(),
        [
            "LiteLLM.AgenticHookError: Exception in async_post_agentic_loop_response_hook [call_id=call-1 model=gpt-4o]: callback test failed: post boom"
        ]
    );
}

#[tokio::test]
async fn the_post_hook_log_names_an_unknown_call_id_without_a_logging_object() {
    let mut parts = followup_parts();
    parts.callback = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(false, false, true))
            .with_post_error("post boom"),
    ));
    let (logs, _guard) = capture();
    let result = execute(&parts.followup_with(LoggingHandle::Absent))
        .await
        .unwrap();
    assert_eq!(response_text(result), MODEL_RESPONSE);
    assert!(logs.messages()[0].contains("[call_id=unknown model=gpt-4o]"));
}

#[tokio::test]
async fn the_post_hook_is_skipped_without_the_override() {
    let mut parts = followup_parts();
    parts.callback = Arc::new(ScriptedCallback::new(
        Script::default().with_post(raw(r#"{"hooked":true}"#)),
    ));
    let result = run(&parts).await.unwrap();
    assert_eq!(response_text(result), MODEL_RESPONSE);
    assert!(parts.callback.records().post_calls.is_empty());
}

#[tokio::test]
async fn the_cleanup_hook_runs_and_its_error_is_swallowed() {
    let mut parts = followup_parts();
    parts.plan.stop_reason = Some("done".to_owned());
    parts.kwargs = json_map(&[("user", "\"u-1\"")]);
    parts.callback = Arc::new(ScriptedCallback::new(
        Script::default().with_cleanup_error("cleanup boom"),
    ));
    let (logs, _guard) = capture();
    let result = run(&parts).await.unwrap();
    assert_eq!(response_text(result), MODEL_RESPONSE);
    let records = parts.callback.records();
    assert_eq!(records.cleanup_calls.len(), 1);
    assert_eq!(
        records.cleanup_calls[0].stop_reason.as_deref(),
        Some("done")
    );
    assert_eq!(
        raw_text(&records.cleanup_calls[0].kwargs, "user").as_deref(),
        Some("\"u-1\"")
    );
    assert_eq!(
        logs.messages(),
        [
            "LiteLLM.AgenticHookError: Exception in async_agentic_loop_cleanup_hook [call_id=call-1 model=gpt-4o]: callback test failed: cleanup boom"
        ]
    );
}

#[tokio::test]
async fn a_continuation_error_reaches_the_caller_after_the_cleanup() {
    let mut parts = followup_parts();
    parts.continuation = FakeContinuation::failing("completion down");
    let result = call_error(run(&parts).await);
    assert!(matches!(&result, ChatCallError::Completion(_)));
    assert_eq!(result.to_string(), "completion down");
    assert_eq!(parts.callback.records().cleanup_calls.len(), 1);
}

#[tokio::test]
async fn missing_patched_messages_fail_before_the_call_and_the_cleanup() {
    let mut parts = followup_parts();
    parts.plan = plan(Some(patch(None)), None, false, true);
    let (logs, _guard) = capture();
    let result = call_error(run(&parts).await);
    assert!(matches!(&result, ChatCallError::MissingMessages));
    assert_eq!(
        result.to_string(),
        "Agentic loop plan missing patched messages"
    );
    assert_eq!(parts.continuation.call_count(), 0);
    assert!(parts.callback.records().cleanup_calls.is_empty());
    assert!(logs.messages().is_empty());
}

#[tokio::test]
async fn a_plan_without_a_patch_has_no_messages_either() {
    let mut parts = followup_parts();
    parts.plan = plan(None, None, false, true);
    let result = call_error(run(&parts).await);
    assert!(matches!(result, ChatCallError::MissingMessages));
    assert_eq!(parts.continuation.call_count(), 0);
    assert!(parts.callback.records().cleanup_calls.is_empty());
}

#[tokio::test]
async fn a_depth_overflow_is_a_typed_error_before_the_cleanup() {
    let mut parts = followup_parts();
    parts.settings.depth = u32::MAX;
    let result = call_error(run(&parts).await);
    assert!(matches!(&result, ChatCallError::Depth(_)));
    assert_eq!(result.to_string(), "agentic loop depth overflowed");
    assert_eq!(parts.continuation.call_count(), 0);
    assert!(parts.callback.records().cleanup_calls.is_empty());
}

#[rstest]
#[case::flag_at_zero("true", 0, true)]
#[case::flag_above_zero("true", 1, false)]
#[case::no_flag_at_zero("false", 0, false)]
#[case::number_flag_at_zero("1", 0, true)]
#[tokio::test]
async fn the_followup_wraps_only_with_the_flag_at_depth_zero(
    #[case] flag: &'static str,
    #[case] depth: u32,
    #[case] wrapped: bool,
) {
    let mut parts = followup_parts();
    parts.kwargs = json_map(&[("_code_interpreter_interception_converted_stream", flag)]);
    parts.settings.depth = depth;
    let result = run(&parts).await.unwrap();
    if wrapped {
        assert_eq!(response_text(result), r#"{"fake_stream":true}"#);
        let records = parts.port.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].response, MODEL_RESPONSE);
        assert_eq!(records[0].model, "gpt-4o");
        assert_eq!(records[0].provider, "openai");
        assert!(records[0].has_logging);
    } else {
        assert_eq!(response_text(result), MODEL_RESPONSE);
        assert!(parts.port.records().is_empty());
    }
}

#[tokio::test]
async fn the_fake_stream_wrap_sees_the_hooked_response() {
    let mut parts = followup_parts();
    parts.kwargs = json_map(&[("_code_interpreter_interception_converted_stream", "true")]);
    parts.callback = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(false, false, true))
            .with_post(raw(HOOKED_RESPONSE)),
    ));
    let result = run(&parts).await.unwrap();
    assert_eq!(response_text(result), r#"{"fake_stream":true}"#);
    assert_eq!(parts.port.records()[0].response, HOOKED_RESPONSE);
}

#[tokio::test]
async fn a_stream_followup_skips_the_post_hook_and_the_wrap() {
    let mut parts = followup_parts();
    parts.kwargs = json_map(&[("_code_interpreter_interception_converted_stream", "true")]);
    parts.continuation = FakeContinuation::responding(ChatLoopResult::Stream(empty_stream()));
    parts.callback = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(false, false, true))
            .with_post(raw(r#"{"hooked":true}"#)),
    ));
    let result = run(&parts).await.unwrap();
    assert!(matches!(result, ChatLoopResult::Stream(_)));
    assert!(parts.callback.records().post_calls.is_empty());
    assert!(parts.port.records().is_empty());
}
