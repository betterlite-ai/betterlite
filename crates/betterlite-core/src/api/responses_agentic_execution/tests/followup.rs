use std::sync::{Arc, Mutex};

use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_models::decisions::Json;

use super::super::dispatch::call_agentic_completion_hooks;
use super::super::executor::{
    AnthropicContinuation, ContinuationExecutor as _, ResponsesContinuation,
};
use super::super::http::HttpExecutorError;
use super::support::{
    NEXT_BODY, PROVIDER_AUTHORIZATION, ScriptedExecutor, SeenRequest, Upstream, callback_list,
    dispatch, fingerprint_of, http_executor, order_log,
};
use super::{
    assert_log, followup_fixture, followup_patch, json, number, plan_callback, response_text, text,
};
use crate::api::chat_agentic_loop::tests::support::capture;
use indexmap::IndexMap;

pub(super) async fn http_followup() {
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
    assert_sent_request(&requests[0]);
    let answer = outcome.expect("the follow-up answer");
    assert_eq!(response_text(answer), NEXT_BODY);
    assert_eq!(callback.posts(), vec![NEXT_BODY.to_owned()]);
    assert_eq!(
        order_log(&order),
        ["f:gate", "f:plan", "f:post", "f:cleanup"]
    );
}

fn assert_sent_request(request: &SeenRequest) {
    assert_eq!(request.request_line, "POST /v1/responses HTTP/1.1");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some(PROVIDER_AUTHORIZATION)
    );
    let Json::Object(fields) = json(&request.body) else {
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
}

pub(super) async fn captured_followup() {
    let (fixture, tools) = followup_fixture();
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let order = Arc::new(Mutex::new(Vec::new()));
    let callback = plan_callback("g", &order, &tools, followup_patch());
    dispatch(&fixture, &callback_list(&[&callback]), &executor)
        .await
        .expect("the fake executor answers");
    let calls = executor.responses_calls();
    assert_eq!(calls.len(), 1);
    assert_continuation(&calls[0], &tools);
}

fn assert_continuation(call: &ResponsesContinuation, tools: &JsonMap) {
    assert_eq!(call.model, "gpt-4o-mini");
    assert_eq!(call.input.len(), 1);
    assert_eq!(
        call.input[0].0.get(),
        r#"{"role":"user","content":"continue"}"#
    );
    assert!(
        matches!(call.params.get("temperature"), Some(Json::Number(value)) if value.to_string() == "0.2")
    );
    assert!(
        matches!(call.params.get("top_p"), Some(Json::Number(value)) if value.to_string() == "0.9")
    );
    assert_eq!(
        call.params.get("tools").cloned(),
        Some(Json::List(Vec::new()))
    );
    assert!(call.params.get("stream").is_none());
    assert!(
        call.params
            .get("_code_interpreter_interception_converted_stream")
            .is_none()
    );
    assert_continuation_kept_kwargs(call);
    assert_continuation_dropped_kwargs(call, tools);
}

fn assert_continuation_kept_kwargs(call: &ResponsesContinuation) {
    assert_eq!(text(&call.kwargs, "unrelated").as_deref(), Some("kept"));
    assert_eq!(text(&call.kwargs, "extra_kw").as_deref(), Some("kept"));
    assert_eq!(text(&call.kwargs, "temperature"), None);
    assert_eq!(
        text(&call.kwargs, "api_base").as_deref(),
        Some("http://provider.invalid/v1")
    );
    assert_eq!(
        number(&call.kwargs, "_code_interpreter_interception_item_parts"),
        Some(3)
    );
    assert_eq!(
        text(&call.kwargs, "_agentic_loop_api_surface").as_deref(),
        Some("responses")
    );
    assert_eq!(
        text(&call.kwargs, "custom_llm_provider").as_deref(),
        Some("openai")
    );
    assert_eq!(
        call.kwargs.get("litellm_logging_obj"),
        None,
        "litellm_logging_obj survived"
    );
    assert_eq!(call.kwargs.get("model"), None, "model survived");
    assert_eq!(call.kwargs.get("input"), None, "input survived");
}

fn assert_continuation_dropped_kwargs(call: &ResponsesContinuation, tools: &JsonMap) {
    let dropped = [
        "_websearch_interception_converted_stream",
        "_compression_interception_item_parts",
        "_headroom_interception_state",
        "_code_interpreter_interception_converted_stream",
    ];
    for key in dropped {
        assert_eq!(call.kwargs.get(key), None, "{key} survived");
    }
    assert_eq!(number(&call.kwargs, "_agentic_loop_depth"), Some(1));
    assert_eq!(number(&call.kwargs, "max_agentic_loops"), Some(2));
    assert_eq!(
        call.kwargs.get("_agentic_loop_fingerprints").cloned(),
        Some(Json::List(vec![
            Json::Text("old".to_owned()),
            Json::Text(fingerprint_of(tools)),
        ]))
    );
}

pub(super) async fn anthropic_surface_is_refused() {
    let continuation = AnthropicContinuation {
        model: "claude-3-5-sonnet".to_owned(),
        messages: Vec::new(),
        max_tokens: 1,
        stream: false,
        params: IndexMap::new(),
    };
    let executor = http_executor("http://provider.invalid");
    let outcome = executor.anthropic_messages(&continuation).await;
    let Err(error) = outcome else {
        panic!("the responses transport refuses the anthropic follow-up");
    };
    assert!(matches!(error, HttpExecutorError::UnsupportedAnthropic));
}

pub(super) async fn failed_send_is_logged() {
    let (fixture, tools) = followup_fixture();
    let failing = ScriptedExecutor::failing("provider exploded");
    let order = Arc::new(Mutex::new(Vec::new()));
    let callback = plan_callback("h", &order, &tools, followup_patch());
    let (log, _guard) = capture();
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
