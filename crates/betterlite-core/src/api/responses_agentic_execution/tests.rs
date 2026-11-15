#![cfg(test)]

pub(crate) mod support;

use std::sync::{Arc, Mutex};

use betterlite_context::agentic_loop_plan::{AgenticLoopRequestPatch, JsonMap};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::values::typed_map;
use super::{AgenticResult, DEFAULT_MAX_TOKENS};
use crate::api::chat_agentic_loop::tests::support::{
    Capture, Script, capture, gate, json_map, overrides, patch, plan, raw,
};
use crate::api::responses_agentic_execution::dispatch::call_agentic_completion_hooks;
use crate::api::responses_agentic_execution::error::{
    AgenticExecutionError, CallbackError, PatchError,
};
use crate::api::responses_agentic_execution::finalize::finalize_refused_response;
use crate::api::responses_hook_policy::error::AgenticLoopSafetyError;
use support::{
    ANTHROPIC_BODY, Fixture, NEXT_BODY, REFUSED_BODY, RecordingCallback, ScriptedExecutor,
    Upstream, anthropic_fixture, callback_list, continue_messages, dispatch, fingerprint_of,
    fixture, http_executor, order_log, plan_harness, plan_with_patch, recording, refused_tools,
    typed_kwargs,
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
    match map.get(key) {
        Some(Json::Number(value)) => value.as_i64(),
        _ => None,
    }
}

fn text(map: &IndexMap<String, Json>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Json::Text(value)) => Some(value.clone()),
        _ => None,
    }
}

fn raw_text(map: &JsonMap, key: &str) -> Option<String> {
    map.get(key).map(|value| value.get().to_owned())
}

fn gated(
    name: &'static str,
    order: &Arc<Mutex<Vec<String>>>,
    tools: &JsonMap,
) -> Arc<RecordingCallback> {
    recording(
        name,
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(true, tools.clone())),
        order,
    )
}

fn legacy(
    name: &'static str,
    order: &Arc<Mutex<Vec<String>>>,
    body: &str,
) -> Arc<RecordingCallback> {
    recording(
        name,
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(true, JsonMap::new()))
            .with_legacy(Some(raw(body))),
        order,
    )
}

fn plan_callback(
    name: &'static str,
    order: &Arc<Mutex<Vec<String>>>,
    tools: &JsonMap,
    request_patch: AgenticLoopRequestPatch,
) -> Arc<RecordingCallback> {
    recording(
        name,
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, tools.clone()))
            .with_plan(plan_with_patch(request_patch)),
        order,
    )
}

#[tokio::test]
async fn agentic_callback_order() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let skipped = recording("skipped", Script::default(), &order);
    let throwing = recording(
        "a",
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate_error("boom"),
        &order,
    );
    let quiet = recording(
        "c",
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(false, JsonMap::new())),
        &order,
    );
    let tools = refused_tools("web_search", "call-ws");
    let follower = plan_callback("b", &order, &tools, patch(Some(continue_messages())));
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let (log, _guard) = capture();
    let outcome = dispatch(
        &fixture(),
        &callback_list(&[&skipped, &throwing, &quiet, &follower]),
        &executor,
    )
    .await
    .expect("the dispatcher runs");
    assert_eq!(
        order_log(&order),
        [
            "a:gate",
            "c:gate",
            "b:gate",
            "b:plan",
            "b:post",
            "b:cleanup"
        ]
    );
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in async_should_run_agentic_loop [call_id=call-1 model=gpt-4o]: boom",
    );
    let records = throwing.records();
    let gate_call = &records.gate[0];
    assert_eq!(
        raw_text(&gate_call.kwargs, "_agentic_loop_api_surface").as_deref(),
        Some("\"responses\"")
    );
    assert_eq!(raw_text(&gate_call.kwargs, "custom_llm_provider"), None);
    let records = follower.records();
    let plan_call = &records.plan[0];
    assert_eq!(
        raw_text(&plan_call.kwargs, "_agentic_loop_api_surface").as_deref(),
        Some("\"responses\"")
    );
    assert_eq!(
        raw_text(&plan_call.kwargs, "custom_llm_provider").as_deref(),
        Some("\"openai\"")
    );
    assert_eq!(plan_call.tools, tools);
    assert_eq!(
        response_text(outcome.expect("the plan path answers")),
        NEXT_BODY
    );

    let order = Arc::new(Mutex::new(Vec::new()));
    let answering = legacy("le", &order, r#"{"legacy":true}"#);
    let never = recording("never", Script::default(), &order);
    let outcome = dispatch(&fixture(), &callback_list(&[&answering, &never]), &executor)
        .await
        .expect("the legacy path answers");
    assert_eq!(
        response_text(outcome.expect("the legacy answer")),
        r#"{"legacy":true}"#
    );
    assert_eq!(order_log(&order), ["le:gate", "le:legacy"]);

    let order = Arc::new(Mutex::new(Vec::new()));
    let silent = recording(
        "si",
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(true, JsonMap::new()))
            .with_legacy(None),
        &order,
    );
    let never = recording("never", Script::default(), &order);
    let outcome = dispatch(&fixture(), &callback_list(&[&silent, &never]), &executor)
        .await
        .expect("a null legacy result is no result");
    assert!(outcome.is_none());
    assert_eq!(order_log(&order), ["si:gate", "si:legacy"]);

    let order = Arc::new(Mutex::new(Vec::new()));
    let broken = recording(
        "br",
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, JsonMap::new()))
            .with_plan_error("plan broke"),
        &order,
    );
    let answering = legacy("le", &order, r#"{"legacy":true}"#);
    let (log, _guard) = capture();
    let outcome = dispatch(
        &fixture(),
        &callback_list(&[&broken, &answering]),
        &executor,
    )
    .await
    .expect("a callback failure is logged, not raised");
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in agentic completion hooks [call_id=call-1 model=gpt-4o]: plan broke",
    );
    assert_eq!(
        order_log(&order),
        ["br:gate", "br:plan", "le:gate", "le:legacy"]
    );
    assert_eq!(
        response_text(outcome.expect("the next callback answers")),
        r#"{"legacy":true}"#
    );

    let outcome = dispatch(&fixture(), &[], &executor)
        .await
        .expect("no callbacks is no result");
    assert!(outcome.is_none());
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
    AgenticLoopRequestPatch {
        model: Some("gpt-4o-mini".to_owned()),
        messages: Some(continue_messages()),
        tools: Some(Vec::new()),
        max_tokens: None,
        optional_params: json_map(&[("top_p", "0.9")]),
        kwargs: json_map(&[("extra_kw", "\"kept\""), ("temperature", "0.5")]),
    }
}

#[tokio::test]
async fn responses_followup_http() {
    let (fixture, tools) = followup_fixture();
    let upstream = Upstream::start().await;
    upstream.answer(200, NEXT_BODY);
    let order = Arc::new(Mutex::new(Vec::new()));
    let callback = plan_callback("f", &order, &tools, followup_patch());
    let outcome = call_agentic_completion_hooks(
        &fixture.call(),
        &callback_list(&[&callback]),
        &http_executor(&upstream.url),
    )
    .await
    .expect("the follow-up send succeeds");
    let requests = upstream.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].request_line, "POST /v1/responses HTTP/1.1");
    let Json::Object(fields) = json(&requests[0].body) else {
        panic!("the follow-up body is an object")
    };
    assert_eq!(text(&fields, "model").as_deref(), Some("gpt-4o-mini"));
    assert_eq!(
        fields.get("input").cloned(),
        Some(json(r#"[{"role":"user","content":"continue"}]"#))
    );
    assert_eq!(fields.get("tools").cloned(), Some(Json::List(Vec::new())));
    assert!(
        matches!(fields.get("temperature"), Some(Json::Number(value)) if value.to_string() == "0.2")
    );
    assert!(fields.get("stream").is_none());
    assert_eq!(
        response_text(outcome.expect("the follow-up answer")),
        NEXT_BODY
    );
    assert_eq!(callback.posts(), vec![NEXT_BODY.to_owned()]);
    assert_eq!(
        order_log(&order),
        ["f:gate", "f:plan", "f:post", "f:cleanup"]
    );

    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let order = Arc::new(Mutex::new(Vec::new()));
    let callback = plan_callback("g", &order, &tools, followup_patch());
    dispatch(&fixture, &callback_list(&[&callback]), &executor)
        .await
        .expect("the fake executor answers");
    let calls = executor.responses_calls();
    assert_eq!(calls.len(), 1);
    let continuation = &calls[0];
    assert_eq!(continuation.model, "gpt-4o-mini");
    assert_eq!(continuation.input.len(), 1);
    assert_eq!(
        continuation.input[0].0.get(),
        r#"{"role":"user","content":"continue"}"#
    );
    assert!(
        matches!(continuation.params.get("temperature"), Some(Json::Number(value)) if value.to_string() == "0.2")
    );
    assert!(
        matches!(continuation.params.get("top_p"), Some(Json::Number(value)) if value.to_string() == "0.9")
    );
    assert_eq!(
        continuation.params.get("tools").cloned(),
        Some(Json::List(Vec::new()))
    );
    assert!(continuation.params.get("stream").is_none());
    assert!(
        continuation
            .params
            .get("_code_interpreter_interception_converted_stream")
            .is_none()
    );
    assert_eq!(
        text(&continuation.kwargs, "unrelated").as_deref(),
        Some("kept")
    );
    assert_eq!(
        text(&continuation.kwargs, "extra_kw").as_deref(),
        Some("kept")
    );
    assert_eq!(text(&continuation.kwargs, "temperature"), None);
    assert_eq!(
        text(&continuation.kwargs, "api_base").as_deref(),
        Some("http://provider.invalid/v1")
    );
    assert_eq!(
        text(&continuation.kwargs, "_agentic_loop_api_surface").as_deref(),
        Some("responses")
    );
    assert_eq!(
        text(&continuation.kwargs, "custom_llm_provider").as_deref(),
        Some("openai")
    );
    assert_eq!(
        continuation.kwargs.get("litellm_logging_obj"),
        None,
        "litellm_logging_obj survived"
    );
    assert_eq!(
        continuation
            .kwargs
            .get("_websearch_interception_converted_stream"),
        None,
        "_websearch_interception_converted_stream survived"
    );
    assert_eq!(
        continuation
            .kwargs
            .get("_compression_interception_item_parts"),
        None,
        "_compression_interception_item_parts survived"
    );
    assert_eq!(
        continuation.kwargs.get("_headroom_interception_state"),
        None,
        "_headroom_interception_state survived"
    );
    assert_eq!(
        continuation
            .kwargs
            .get("_code_interpreter_interception_converted_stream"),
        None,
        "_code_interpreter_interception_converted_stream survived"
    );
    assert_eq!(continuation.kwargs.get("model"), None, "model survived");
    assert_eq!(continuation.kwargs.get("input"), None, "input survived");
    assert_eq!(number(&continuation.kwargs, "_agentic_loop_depth"), Some(1));
    assert_eq!(number(&continuation.kwargs, "max_agentic_loops"), Some(2));
    assert_eq!(
        continuation
            .kwargs
            .get("_agentic_loop_fingerprints")
            .cloned(),
        Some(Json::List(vec![
            Json::Text("old".to_owned()),
            Json::Text(fingerprint_of(&tools)),
        ]))
    );

    let failing = ScriptedExecutor::failing("provider exploded");
    let (log, _guard) = capture();
    let order = Arc::new(Mutex::new(Vec::new()));
    let callback = plan_callback("h", &order, &tools, followup_patch());
    let outcome = dispatch(&fixture, &callback_list(&[&callback]), &failing)
        .await
        .expect("a failed send is logged, not raised");
    assert!(outcome.is_none());
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in agentic completion hooks [call_id=call-1 model=gpt-4o]: provider exploded",
    );
    assert_eq!(order_log(&order), ["h:gate", "h:plan", "h:cleanup"]);
}

#[tokio::test]
async fn agentic_override_termination_and_safety() {
    let tools = refused_tools("web_search", "call-ws");
    let order = Arc::new(Mutex::new(Vec::new()));
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let overridden = recording(
        "o",
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, tools.clone()))
            .with_plan(plan(None, Some(raw("false")), true, false)),
        &order,
    );
    let outcome = dispatch(&fixture(), &callback_list(&[&overridden]), &executor)
        .await
        .expect("the override answers");
    assert_eq!(
        response_text(outcome.expect("an override is a result")),
        "false"
    );
    assert!(overridden.posts().is_empty());
    assert_eq!(executor.call_count(), 0);

    let mut terminate = plan(None, None, true, false);
    terminate.stop_reason = Some("done".to_owned());
    let order = Arc::new(Mutex::new(Vec::new()));
    let terminated = recording(
        "t",
        Script::default()
            .with_overrides(overrides(true, true, true))
            .with_gate(gate(true, tools.clone()))
            .with_plan(terminate)
            .with_post(raw(r#"{"post":"replaced"}"#)),
        &order,
    );
    let mut refused = fixture();
    refused.response = raw(REFUSED_BODY);
    let (log, _guard) = capture();
    let outcome = dispatch(&refused, &callback_list(&[&terminated]), &executor)
        .await
        .expect("the terminate path answers");
    assert_log(&log, "Agentic loop terminated by callback=t reason=done");
    assert_eq!(order_log(&order), ["t:gate", "t:plan", "t:post"]);
    assert_eq!(
        response_text(outcome.expect("the post answer")),
        r#"{"post":"replaced"}"#
    );
    assert_eq!(
        json(&terminated.posts()[0]),
        json(
            r#"{"id":"msg_2","type":"message","role":"assistant","content":[{"type":"text","text":"after"}],"stop_reason":"end_turn"}"#
        )
    );

    let tools_json = Json::Object(typed_map(&tools).expect("the tool map"));
    assert_eq!(
        finalize_refused_response(&raw(r#"[1,2]"#), &tools_json).expect("a list survives"),
        json("[1,2]")
    );
    assert_eq!(
        finalize_refused_response(&raw(r#"{"id":"x"}"#), &tools_json)
            .expect("a dict without content survives"),
        json(r#"{"id":"x"}"#)
    );

    let order = Arc::new(Mutex::new(Vec::new()));
    let post_failing = recording(
        "e",
        Script::default()
            .with_overrides(overrides(true, true, true))
            .with_gate(gate(true, tools.clone()))
            .with_plan(plan(None, None, true, false))
            .with_post_error("post broke"),
        &order,
    );
    let quiet = recording(
        "q",
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(false, JsonMap::new())),
        &order,
    );
    let (log, _guard) = capture();
    let outcome = dispatch(
        &refused,
        &callback_list(&[&post_failing, &quiet]),
        &executor,
    )
    .await
    .expect("a terminate post failure is caught per callback");
    assert!(outcome.is_none());
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in async_post_agentic_loop_response_hook [call_id=call-1 model=gpt-4o]: post broke",
    );
    assert_eq!(order_log(&order), ["e:gate", "e:plan", "e:post", "q:gate"]);

    let order = Arc::new(Mutex::new(Vec::new()));
    let mut deep = fixture();
    deep.kwargs = typed_kwargs(&[("_agentic_loop_depth", "1"), ("max_agentic_loops", "1")]);
    let error = dispatch_error(
        dispatch(
            &deep,
            &callback_list(&[&gated("s", &order, &tools)]),
            &executor,
        )
        .await,
        "a responses refusal raises",
    );
    assert!(matches!(
        error,
        AgenticExecutionError::Safety(AgenticLoopSafetyError::MaxLoopsExceeded { .. })
    ));

    let mut repeated = fixture();
    repeated.kwargs.insert(
        "_agentic_loop_fingerprints".to_owned(),
        Json::List(vec![Json::Text(fingerprint_of(&tools))]),
    );
    let error = dispatch_error(
        dispatch(
            &repeated,
            &callback_list(&[&gated("s", &order, &tools)]),
            &executor,
        )
        .await,
        "a repeated fingerprint raises",
    );
    assert!(matches!(
        error,
        AgenticExecutionError::Safety(AgenticLoopSafetyError::RepeatedFingerprint)
    ));

    let mut refused = anthropic_fixture();
    refused.response = raw(REFUSED_BODY);
    refused.kwargs = typed_kwargs(&[("_agentic_loop_depth", "1"), ("max_agentic_loops", "1")]);
    let (log, _guard) = capture();
    let outcome = dispatch(
        &refused,
        &callback_list(&[&gated("s", &order, &tools)]),
        &executor,
    )
    .await
    .expect("a non-streaming anthropic refusal ends the turn");
    assert_log(
        &log,
        "LiteLLM.AgenticLoopRefused: ending turn [call_id=call-1 model=claude-3-5-sonnet]: Exceeded max_agentic_loops=1 for model=claude-3-5-sonnet",
    );
    assert_eq!(
        json(&response_text(outcome.expect("the finalized turn"))),
        json(
            r#"{"id":"msg_2","type":"message","role":"assistant","content":[{"type":"text","text":"after"}],"stop_reason":"end_turn"}"#
        )
    );

    let mut streaming = anthropic_fixture();
    streaming.response = raw(REFUSED_BODY);
    streaming.stream = true;
    streaming.kwargs = typed_kwargs(&[("_agentic_loop_depth", "1"), ("max_agentic_loops", "1")]);
    let error = dispatch_error(
        dispatch(
            &streaming,
            &callback_list(&[&gated("s", &order, &tools)]),
            &executor,
        )
        .await,
        "a streaming anthropic refusal raises",
    );
    assert!(matches!(
        error,
        AgenticExecutionError::Safety(AgenticLoopSafetyError::MaxLoopsExceeded { .. })
    ));

    let mut client_kept = anthropic_fixture();
    client_kept.response = raw(
        r#"{"id":"msg_3","type":"message","role":"assistant","content":[{"type":"tool_use","id":"call-ws","name":"web_search","input":{}},{"type":"tool_use","id":"call-client","name":"search","input":{}}],"stop_reason":"tool_use"}"#,
    );
    client_kept.kwargs = typed_kwargs(&[("_agentic_loop_depth", "1"), ("max_agentic_loops", "1")]);
    let outcome = dispatch(
        &client_kept,
        &callback_list(&[&gated("s", &order, &tools)]),
        &executor,
    )
    .await
    .expect("a client tool call keeps the turn open");
    assert_eq!(
        json(&response_text(outcome.expect("the finalized turn"))),
        json(
            r#"{"id":"msg_3","type":"message","role":"assistant","content":[{"type":"tool_use","id":"call-client","name":"search","input":{}}],"stop_reason":"tool_use"}"#
        )
    );
}

#[tokio::test]
async fn continuation_patch_failures() {
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let harness = plan_harness(fixture(), Script::default());
    let missing = plan(Some(patch(None)), None, false, true);
    let error = harness
        .responses(&missing, &executor)
        .await
        .expect_err("a responses plan without input raises");
    assert!(matches!(
        error,
        CallbackError::Patch(PatchError::MissingResponsesInput)
    ));
    assert!(harness.callback.records().cleanup.is_empty());
    assert_eq!(executor.call_count(), 0);

    let error = harness
        .anthropic(
            &missing,
            &ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY),
        )
        .await
        .expect_err("an anthropic plan without messages raises");
    assert!(matches!(
        error,
        CallbackError::Patch(PatchError::MissingMessages)
    ));
    assert!(harness.callback.records().cleanup.is_empty());

    let valid = plan_with_patch(patch(Some(continue_messages())));
    let harness = plan_harness(fixture(), Script::default());
    let error = harness
        .responses(&valid, &ScriptedExecutor::failing("provider exploded"))
        .await
        .expect_err("a provider failure raises");
    match error {
        CallbackError::Execution(source) => assert_eq!(source.message, "provider exploded"),
        other => panic!("expected the provider failure, got {other}"),
    }
    let records = harness.callback.records();
    assert_eq!(records.cleanup.len(), 1);
    assert_eq!(records.post.len(), 0);

    let harness = plan_harness(fixture(), Script::default().with_post_error("post broke"));
    let (log, _guard) = capture();
    let response = harness
        .responses(&valid, &ScriptedExecutor::responding(NEXT_BODY))
        .await
        .expect("a post failure keeps the provider answer");
    assert_eq!(response.get(), NEXT_BODY);
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in async_post_agentic_loop_response_hook [call_id=call-1 model=gpt-4o]: post broke",
    );
    assert_eq!(harness.callback.records().cleanup.len(), 1);

    let mut silent = plan_harness(
        fixture(),
        Script::default().with_cleanup_error("cleanup broke"),
    );
    silent.logging = None;
    let (log, _guard) = capture();
    let response = silent
        .responses(&valid, &ScriptedExecutor::responding(NEXT_BODY))
        .await
        .expect("a cleanup failure keeps the provider answer");
    assert_eq!(response.get(), NEXT_BODY);
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in async_agentic_loop_cleanup_hook [call_id=unknown model=gpt-4o]: cleanup broke",
    );

    let mut silent = plan_harness(
        fixture(),
        Script::default().with_cleanup_error("cleanup broke"),
    );
    silent.logging = None;
    let (log, _guard) = capture();
    let error = silent
        .responses(&valid, &ScriptedExecutor::failing("provider exploded"))
        .await
        .expect_err("cleanup never masks the provider failure");
    match error {
        CallbackError::Execution(source) => assert_eq!(source.message, "provider exploded"),
        other => panic!("expected the provider failure, got {other}"),
    }
    assert_log(
        &log,
        "async_agentic_loop_cleanup_hook [call_id=unknown model=gpt-4o]: cleanup broke",
    );

    let order = Arc::new(Mutex::new(Vec::new()));
    let broken = recording(
        "br",
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, JsonMap::new()))
            .with_plan(plan(Some(patch(None)), None, false, true)),
        &order,
    );
    let answering = legacy("le", &order, r#"{"legacy":true}"#);
    let (log, _guard) = capture();
    let outcome = dispatch(
        &fixture(),
        &callback_list(&[&broken, &answering]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("a plan failure is logged and the next callback runs");
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in agentic completion hooks [call_id=call-1 model=gpt-4o]: Agentic loop plan missing patched responses input",
    );
    assert_eq!(
        response_text(outcome.expect("the next callback answers")),
        r#"{"legacy":true}"#
    );

    let (log, _guard) = capture();
    let outcome = dispatch(
        &anthropic_fixture(),
        &callback_list(&[&broken]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("an anthropic plan failure is logged");
    assert!(outcome.is_none());
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in agentic completion hooks [call_id=call-1 model=claude-3-5-sonnet]: Agentic loop plan missing patched messages",
    );
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
async fn anthropic_continuation_and_wrapping() {
    let mut logged = anthropic_fixture();
    logged.agentic_loop_params = Some(json_map(&[("model", "\"logged-model\"")]));
    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    plan_harness(logged.clone(), Script::default())
        .anthropic(&plan_with_patch(patched()), &executor)
        .await
        .expect("the patch model is used");
    assert_eq!(executor.anthropic_calls()[0].model, "patched-model");

    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    plan_harness(logged, Script::default())
        .anthropic(&plan_with_patch(without_model(&patched())), &executor)
        .await
        .expect("the logged model is used");
    assert_eq!(executor.anthropic_calls()[0].model, "logged-model");

    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    plan_harness(anthropic_fixture(), Script::default())
        .anthropic(&plan_with_patch(without_model(&patched())), &executor)
        .await
        .expect("the request model is used");
    assert_eq!(executor.anthropic_calls()[0].model, "claude-3-5-sonnet");

    let mut ladder = anthropic_fixture();
    ladder.optional_params = json_map(&[("max_tokens", "500")]);
    ladder.kwargs = typed_kwargs(&[("max_tokens", "900")]);
    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    plan_harness(ladder.clone(), Script::default())
        .anthropic(
            &plan_with_patch(AgenticLoopRequestPatch {
                max_tokens: Some(700),
                ..without_model(&patched())
            }),
            &executor,
        )
        .await
        .expect("the merged mapping's max_tokens wins");
    assert_eq!(executor.anthropic_calls()[0].max_tokens, 900);

    ladder.kwargs = IndexMap::new();
    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    plan_harness(ladder.clone(), Script::default())
        .anthropic(
            &plan_with_patch(AgenticLoopRequestPatch {
                max_tokens: Some(700),
                ..without_model(&patched())
            }),
            &executor,
        )
        .await
        .expect("the patch max_tokens wins over the optional one");
    assert_eq!(executor.anthropic_calls()[0].max_tokens, 700);

    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    plan_harness(ladder, Script::default())
        .anthropic(&plan_with_patch(without_model(&patched())), &executor)
        .await
        .expect("the optional max_tokens is used");
    let calls = executor.anthropic_calls();
    assert_eq!(calls[0].max_tokens, 500);
    assert!(calls[0].params.get("max_tokens").is_none());

    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    let mut only_kwargs = anthropic_fixture();
    only_kwargs.kwargs = typed_kwargs(&[("max_tokens", "900")]);
    plan_harness(only_kwargs, Script::default())
        .anthropic(&plan_with_patch(without_model(&patched())), &executor)
        .await
        .expect("the kwargs max_tokens is used");
    assert_eq!(executor.anthropic_calls()[0].max_tokens, 900);

    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    plan_harness(anthropic_fixture(), Script::default())
        .anthropic(&plan_with_patch(without_model(&patched())), &executor)
        .await
        .expect("the default max_tokens is used");
    assert_eq!(executor.anthropic_calls()[0].max_tokens, DEFAULT_MAX_TOKENS);

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
    let executor = ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY);
    let mut harness = plan_harness(merged, Script::default());
    harness.settings.fingerprints = vec![Json::Text("old".to_owned())];
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
    harness
        .anthropic(&plan_with_patch(overlay), &executor)
        .await
        .expect("the merged mapping is captured");
    let calls = executor.anthropic_calls();
    let call = &calls[0];
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

    let order = Arc::new(Mutex::new(Vec::new()));
    let quiet = recording(
        "w",
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(false, JsonMap::new())),
        &order,
    );
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

    converted.converted = false;
    let outcome = dispatch(
        &converted,
        &callback_list(&[&quiet]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("an unconverted anthropic answer keeps the response");
    assert!(outcome.is_none());

    let mut responses_converted = fixture();
    responses_converted.converted = true;
    let outcome = dispatch(
        &responses_converted,
        &callback_list(&[&quiet]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("the responses surface is never wrapped");
    assert!(outcome.is_none());
}
