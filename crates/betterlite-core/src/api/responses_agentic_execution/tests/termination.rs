use std::sync::{Arc, Mutex};

use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_models::decisions::Json;

use super::super::error::AgenticExecutionError;
use super::super::finalize::finalize_refused_response;
use super::super::values::typed_map;
use super::support::{
    Fixture, NEXT_BODY, REFUSED_BODY, ScriptedExecutor, anthropic_fixture, callback_list, dispatch,
    fingerprint_of, fixture, order_log, recording, refused_tools, typed_kwargs,
};
use super::{assert_log, dispatch_error, gated, json, response_text};
use crate::api::chat_agentic_loop::tests::support::{Script, capture, gate, overrides, plan, raw};
use crate::api::responses_hook_policy::error::AgenticLoopSafetyError;

pub(super) async fn override_answers() {
    let tools = refused_tools("web_search", "call-ws");
    let order = Arc::new(Mutex::new(Vec::new()));
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let overridden = recording(
        "o",
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, tools))
            .with_plan(plan(None, Some(raw("false")), true, false)),
        &order,
    );
    let outcome = dispatch(&fixture(), &callback_list(&[&overridden]), &executor)
        .await
        .expect("the override answers");
    let answer = outcome.expect("an override is a result");
    assert_eq!(response_text(answer), "false");
    assert!(overridden.posts().is_empty());
    assert_eq!(executor.call_count(), 0);
}

pub(super) async fn terminate_posts() {
    let tools = refused_tools("web_search", "call-ws");
    let mut terminate = plan(None, None, true, false);
    terminate.stop_reason = Some("done".to_owned());
    let order = Arc::new(Mutex::new(Vec::new()));
    let terminated = recording(
        "t",
        Script::default()
            .with_overrides(overrides(true, true, true))
            .with_gate(gate(true, tools))
            .with_plan(terminate)
            .with_post(raw(r#"{"post":"replaced"}"#)),
        &order,
    );
    let mut refused = fixture();
    refused.response = raw(REFUSED_BODY);
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let (log, _guard) = capture();
    let outcome = dispatch(&refused, &callback_list(&[&terminated]), &executor)
        .await
        .expect("the terminate path answers");
    assert_log(&log, "Agentic loop terminated by callback=t reason=done");
    assert_eq!(order_log(&order), ["t:gate", "t:plan", "t:post"]);
    let answer = outcome.expect("the post answer");
    assert_eq!(response_text(answer), r#"{"post":"replaced"}"#);
    assert_eq!(
        json(&terminated.posts()[0]),
        json(
            r#"{"id":"msg_2","type":"message","role":"assistant","content":[{"type":"text","text":"after"}],"stop_reason":"end_turn"}"#
        )
    );
}

pub(super) fn non_object_finalize_keeps_value() {
    let tools =
        Json::Object(typed_map(&refused_tools("web_search", "call-ws")).expect("the tool map"));
    assert_eq!(
        finalize_refused_response(&raw("[1,2]"), &tools).expect("a list survives"),
        json("[1,2]")
    );
    assert_eq!(
        finalize_refused_response(&raw(r#"{"id":"x"}"#), &tools)
            .expect("a dict without content survives"),
        json(r#"{"id":"x"}"#)
    );
}

pub(super) async fn post_failure_caught() {
    let tools = refused_tools("web_search", "call-ws");
    let order = Arc::new(Mutex::new(Vec::new()));
    let post_failing = recording(
        "e",
        Script::default()
            .with_overrides(overrides(true, true, true))
            .with_gate(gate(true, tools))
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
    let mut refused = fixture();
    refused.response = raw(REFUSED_BODY);
    let executor = ScriptedExecutor::responding(NEXT_BODY);
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
        "LiteLLM.AgenticHookError: Exception in agentic completion hooks [call_id=call-1 model=gpt-4o]: post broke",
    );
    assert_eq!(order_log(&order), ["e:gate", "e:plan", "e:post", "q:gate"]);
}

async fn refuses_with_max_loops(fixture: &Fixture) {
    let tools = refused_tools("web_search", "call-ws");
    let order = Arc::new(Mutex::new(Vec::new()));
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let error = dispatch_error(
        dispatch(
            fixture,
            &callback_list(&[&gated("s", &order, &tools)]),
            &executor,
        )
        .await,
        "the refusal raises",
    );
    assert!(matches!(
        error,
        AgenticExecutionError::Safety(AgenticLoopSafetyError::MaxLoopsExceeded { .. })
    ));
}

pub(super) async fn max_loops_refuses_responses() {
    let mut deep = fixture();
    deep.kwargs = typed_kwargs(&[("_agentic_loop_depth", "1"), ("max_agentic_loops", "1")]);
    refuses_with_max_loops(&deep).await;
}

pub(super) async fn streaming_refusal_raises() {
    let mut streaming = anthropic_fixture();
    streaming.response = raw(REFUSED_BODY);
    streaming.stream = true;
    streaming.kwargs = typed_kwargs(&[("_agentic_loop_depth", "1"), ("max_agentic_loops", "1")]);
    refuses_with_max_loops(&streaming).await;
}

pub(super) async fn repeated_fingerprint_refuses() {
    let tools = refused_tools("web_search", "call-ws");
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut repeated = fixture();
    repeated.kwargs.insert(
        "_agentic_loop_fingerprints".to_owned(),
        Json::List(vec![Json::Text(fingerprint_of(&tools))]),
    );
    let executor = ScriptedExecutor::responding(NEXT_BODY);
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
}

pub(super) async fn anthropic_refusal_finalizes() {
    let tools = refused_tools("web_search", "call-ws");
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut refused = anthropic_fixture();
    refused.response = raw(REFUSED_BODY);
    refused.kwargs = typed_kwargs(&[("_agentic_loop_depth", "1"), ("max_agentic_loops", "1")]);
    let executor = ScriptedExecutor::responding(NEXT_BODY);
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
    let answer = outcome.expect("the finalized turn");
    assert_eq!(
        json(&response_text(answer)),
        json(
            r#"{"id":"msg_2","type":"message","role":"assistant","content":[{"type":"text","text":"after"}],"stop_reason":"end_turn"}"#
        )
    );
}

pub(super) async fn client_tool_use_kept() {
    let tools = refused_tools("web_search", "call-ws");
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut client_kept = anthropic_fixture();
    client_kept.response = raw(
        r#"{"id":"msg_3","type":"message","role":"assistant","content":[{"type":"tool_use","id":"call-ws","name":"web_search","input":{}},{"type":"tool_use","id":"call-client","name":"search","input":{}}],"stop_reason":"tool_use"}"#,
    );
    client_kept.kwargs = typed_kwargs(&[("_agentic_loop_depth", "1"), ("max_agentic_loops", "1")]);
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let outcome = dispatch(
        &client_kept,
        &callback_list(&[&gated("s", &order, &tools)]),
        &executor,
    )
    .await
    .expect("a client tool call keeps the turn open");
    let answer = outcome.expect("the finalized turn");
    assert_eq!(
        json(&response_text(answer)),
        json(
            r#"{"id":"msg_3","type":"message","role":"assistant","content":[{"type":"tool_use","id":"call-client","name":"search","input":{}}],"stop_reason":"tool_use"}"#
        )
    );
}
