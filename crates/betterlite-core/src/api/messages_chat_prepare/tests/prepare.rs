use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::summary_model_access::Settings;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_models::decisions::Json;

use crate::api::messages_chat_prepare::inspect::extract_proxy_metadata;
use crate::api::messages_chat_prepare::prepare::{
    ContextRequest, DispatchFailure, prepare_context_managed_request,
};
use crate::api::messages_compaction::editor::RegisteredEditors;
use crate::api::messages_context_clear::constants::COMPACT_EDIT_TYPE;
use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::summary_execution::completion::{ExecutorCompletion, SummaryCompletion};

use super::support::{
    DispatchRecord, FixedClearCounter, Logs, NeverCalled, NoBudgets, NoChecks, NoHooks, NoRows,
    Recorder, RecordingHistoryCounter, RouterCompletion, ScriptedDispatch, capture_logs,
    context_failure, decided, deployment, executor, extra, parse, polyfilled, principal,
    text_field,
};

const SUMMARY_BODY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":0,"model":"gpt-4o-mini","choices":[{"index":0,"message":{"role":"assistant","content":"<summary>N</summary>"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}}"#;
const HISTORY: &str = r#"[
    {"role":"user","content":"old question"},
    {"role":"assistant","content":[{"type":"compaction","content":"S"}]},
    {"role":"assistant","content":[{"type":"tool_use","id":"tool_1","name":"lookup","input":{}}]},
    {"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_1","content":"recent result"}]},
    {"role":"user","content":"recent question"}
]"#;
const TAIL: &str = r#"[
    {"role":"assistant","content":[{"type":"tool_use","id":"tool_1","name":"lookup","input":{}}]},
    {"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_1","content":"recent result"}]},
    {"role":"user","content":"recent question"}
]"#;
const COMPACT_HISTORY: &str = r#"[
    {"role":"user","content":"old question"},
    {"role":"assistant","content":"old answer"},
    {"role":"assistant","content":[{"type":"compaction","content":"S"}]},
    {"role":"user","content":"recent question one"},
    {"role":"assistant","content":"recent answer one"},
    {"role":"user","content":"recent question two"}
]"#;
const LAST_QUESTION: &str = r#"[{"role":"user","content":"recent question two"}]"#;
const TOOLS: &str = r#"[{"name":"lookup","input_schema":{"type":"object"}}]"#;
const METADATA: &str = r#"{"user_id":"u-1","litellm_call_id":"c-1"}"#;
const KWARGS: &str = r#"{"litellm_metadata":{"user_id":"u-1","litellm_call_id":"c-1"}}"#;
const PRINCIPAL: &str = r#"{"api_key":"sk-1","user_id":"u-1"}"#;
const CLEAR_SPEC: &str = r#"{"edits":[{"type":"clear_tool_uses_20250919","trigger":{"type":"input_tokens","value":50000}}]}"#;
const COMPACT_SPEC: &str =
    r#"{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":50000}}]}"#;
const LOW_TRIGGER: &str =
    r#"{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":40000}}]}"#;
const AUGMENTED: &str = r#""Previous conversation summary: S\n\nBASE SYSTEM""#;
const SCRIPTED_SYSTEM: &str = r#""SCRIPTED SYSTEM""#;
const SCRIPTED: &str = r#"[{"role":"assistant","content":"scripted"}]"#;
const PLAIN: &str = r#"[{"role":"user","content":"hello"}]"#;

fn context<'request>(
    messages: &'request [Json],
    system: Option<&'request Json>,
    spec: Option<&'request Json>,
    drops: Option<&'request Json>,
    tools: Option<&'request [Json]>,
) -> ContextRequest<'request> {
    ContextRequest {
        model: "main-model",
        messages,
        tools,
        system,
        context_management_spec: spec,
        additional_drop_params: drops,
        litellm_metadata: None,
        llm_router: None,
        user_api_key_auth: None,
    }
}

fn history_fixture() -> (Vec<Json>, Json, Vec<Json>) {
    (decided(HISTORY), parse(r#""BASE SYSTEM""#), decided(TOOLS))
}

fn history_result() -> PolyfillResult {
    polyfilled(decided(TAIL), Some(parse(AUGMENTED)))
}

fn summary_settings() -> Settings {
    Settings {
        context_management_summary_model: Some(String::from("gpt-4o-mini")),
        context_management_summary_max_tokens: 4096,
    }
}

fn serve_summary() -> Serving {
    serve(
        reply(
            "200 OK",
            &[("content-type", "application/json")],
            SUMMARY_BODY,
        ),
        Duration::ZERO,
    )
    .expect("the fixture serves")
}

fn editors<'services, Completion>(
    settings: Option<&'services Settings>,
    counter: &'services RecordingHistoryCounter,
    completion: &'services Completion,
) -> RegisteredEditors<'services, NoChecks, NoRows, NoBudgets, NoHooks, RecordingHistoryCounter>
where
    Completion: SummaryCompletion,
{
    RegisteredEditors {
        settings,
        proxy: None,
        budgets: None,
        hooks: None,
        history_counter: counter,
        clear_counter: Arc::new(FixedClearCounter { count: 0 }),
        direct: completion,
    }
}

fn max_tokens(body: &Json) -> u64 {
    let Json::Object(fields) = body else {
        panic!("the summary request body is an object");
    };
    match fields.get("max_tokens") {
        Some(Json::Number(number)) => number.as_u64().expect("max_tokens is an integer"),
        other => panic!("the summary request carries max_tokens: {other:?}"),
    }
}

#[tokio::test]
async fn client_history_before_noncompact_edit() {
    let (messages, system, tools) = history_fixture();
    let spec = parse(CLEAR_SPEC);
    let answer = polyfilled(decided(SCRIPTED), Some(parse(SCRIPTED_SYSTEM)));
    let dispatcher = ScriptedDispatch::new(Ok(Some(answer.clone())));
    let result = prepare_context_managed_request(
        context(&messages, Some(&system), Some(&spec), None, Some(&tools)),
        &dispatcher,
    )
    .await
    .expect("the wrapper prepares");
    assert_eq!(result, Some(answer));
    assert_clear_input(&dispatcher.calls(), &spec, &tools);
    assert_disabled_forwards(&messages, &system, &spec, &tools).await;
    assert_no_block_forwards_nothing(&system, &spec).await;
    assert_empty_edits_runs(&messages, &system, &tools).await;
}

fn assert_clear_input(calls: &[DispatchRecord], spec: &Json, tools: &[Json]) {
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "main-model");
    assert_eq!(calls[0].messages, decided(TAIL));
    assert_eq!(calls[0].system, Some(parse(AUGMENTED)));
    assert_eq!(calls[0].tools.as_deref(), Some(tools));
    assert_eq!(calls[0].spec.as_ref(), Some(spec));
}

async fn assert_disabled_forwards(messages: &[Json], system: &Json, spec: &Json, tools: &[Json]) {
    let history = history_result();
    let disabled = ScriptedDispatch::new(Ok(None));
    for candidate in [None, Some(parse("null")), Some(parse("{}"))] {
        let result = prepare_context_managed_request(
            context(
                messages,
                Some(system),
                candidate.as_ref(),
                None,
                Some(tools),
            ),
            &disabled,
        )
        .await
        .expect("a disabled spec still forwards history");
        assert_eq!(result, Some(history.clone()));
    }
    let drops = parse(r#"["context_management"]"#);
    let result = prepare_context_managed_request(
        context(
            messages,
            Some(system),
            Some(spec),
            Some(&drops),
            Some(tools),
        ),
        &disabled,
    )
    .await
    .expect("a dropped spec still forwards history");
    assert_eq!(result, Some(history));
    assert!(disabled.calls().is_empty());
}

// an object spec is truthy however empty its edits are, so it reaches the dispatcher and its result wins
async fn assert_empty_edits_runs(messages: &[Json], system: &Json, tools: &[Json]) {
    let spec = parse(r#"{"edits":[]}"#);
    let answer = polyfilled(decided(SCRIPTED), Some(parse(SCRIPTED_SYSTEM)));
    let dispatcher = ScriptedDispatch::new(Ok(Some(answer.clone())));
    let result = prepare_context_managed_request(
        context(messages, Some(system), Some(&spec), None, Some(tools)),
        &dispatcher,
    )
    .await
    .expect("an empty edits object still runs");
    assert_eq!(result, Some(answer));
    assert_eq!(dispatcher.calls().len(), 1);
}

async fn assert_no_block_forwards_nothing(system: &Json, spec: &Json) {
    let plain = decided(PLAIN);
    let drops = parse(r#"["context_management"]"#);
    let dispatcher = ScriptedDispatch::new(Ok(None));
    let result = prepare_context_managed_request(
        context(&plain, Some(system), Some(spec), Some(&drops), None),
        &dispatcher,
    )
    .await
    .expect("nothing to forward");
    assert!(result.is_none());
    assert!(dispatcher.calls().is_empty());
}

#[tokio::test]
async fn compact_editor_receives_full_input() {
    let spec = parse(COMPACT_SPEC);
    let messages = decided(COMPACT_HISTORY);
    let system = parse(r#""BASE SYSTEM""#);
    let tools = decided(TOOLS);
    let proxy = extract_proxy_metadata(&extra(KWARGS)).expect("the proxy context");
    let auth = principal(PRINCIPAL);
    let counter = RecordingHistoryCounter::new(50_001);
    let serving = serve_summary();
    let executor = executor();
    let summary_deployment = deployment(&format!("http://{}/v1", serving.address));
    let router = RouterCompletion {
        inner: ExecutorCompletion {
            executor: &executor,
            deployment: &summary_deployment,
        },
    };
    let direct = NeverCalled;
    let settings = summary_settings();
    let dispatcher = Recorder::new(editors(Some(&settings), &counter, &direct));
    let request = ContextRequest {
        litellm_metadata: Some(&proxy.litellm_metadata),
        llm_router: Some(&router),
        user_api_key_auth: Some(&auth),
        ..context(&messages, Some(&system), Some(&spec), None, Some(&tools))
    };
    let result = prepare_context_managed_request(request, &dispatcher)
        .await
        .expect("the compact editor runs")
        .expect("a compact result");
    assert_editor_input(&dispatcher.calls(), &spec, &messages, &system, &tools);
    assert_counted(&counter.seen());
    assert_compact_result(&result);
    assert_summary_request(&serving.finish().expect("one summary request"));
}

fn assert_editor_input(
    calls: &[DispatchRecord],
    spec: &Json,
    messages: &[Json],
    system: &Json,
    tools: &[Json],
) {
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "main-model");
    assert_eq!(calls[0].messages, messages);
    assert_eq!(calls[0].system, Some(system.clone()));
    assert_eq!(calls[0].tools.as_deref(), Some(tools));
    assert_eq!(calls[0].spec.as_ref(), Some(spec));
    assert_eq!(
        serde_json::to_value(
            calls[0]
                .metadata
                .as_ref()
                .expect("the metadata is passed on")
        )
        .expect("it encodes"),
        parse(METADATA)
    );
    assert!(calls[0].has_router);
    assert!(calls[0].has_principal);
}

fn assert_counted(counted: &[Json]) {
    let text: String = counted
        .iter()
        .map(|entry| serde_json::to_string(entry).expect("the counted conversation encodes"))
        .collect::<Vec<String>>()
        .join("\n");
    assert!(
        text.contains("recent question one"),
        "the first tail turn reaches the count: {text}"
    );
    assert!(
        text.contains("recent question two"),
        "the last tail turn reaches the count: {text}"
    );
    assert!(
        !text.contains("old question"),
        "the pre-block history stays out of the count: {text}"
    );
}

fn assert_compact_result(result: &PolyfillResult) {
    assert_eq!(result.messages, decided(LAST_QUESTION));
    assert_eq!(
        result.system,
        Some(parse(
            r#""Previous conversation summary: N\n\nBASE SYSTEM""#
        ))
    );
    let block = result
        .compaction_block
        .as_ref()
        .expect("a compaction block");
    assert_eq!(block.content.as_deref(), Some("N"));
    assert_eq!(result.applied_edits.len(), 1);
    assert_eq!(
        result.applied_edits[0].edit_type.as_deref(),
        Some(COMPACT_EDIT_TYPE)
    );
    assert_eq!(result.applied_edits[0].summary_input_tokens, Some(12));
    assert_eq!(result.applied_edits[0].summary_output_tokens, Some(3));
}

fn assert_summary_request(request: &Request) {
    assert_eq!(request.line, "POST /v1/chat/completions HTTP/1.1");
    let body = parse(&request.body);
    assert_eq!(text_field(&body, "model"), Some("gpt-4o-mini"));
    assert_eq!(max_tokens(&body), 4096);
    assert!(
        request.body.contains("recent question one"),
        "the tail reaches the summary call: {}",
        request.body
    );
}

#[tokio::test]
async fn compact_failure_uses_history_safety_net() {
    let (messages, system, _tools) = history_fixture();
    let spec = parse(COMPACT_SPEC);
    let (logs, _guard) = capture_logs();
    let broken = ScriptedDispatch::new(Err(DispatchFailure::Other(String::from("broken"))));
    let result = prepare_context_managed_request(
        context(&messages, Some(&system), Some(&spec), None, None),
        &broken,
    )
    .await
    .expect("a compact-only failure is swallowed");
    assert_eq!(result, Some(history_result()));
    assert_skipping_diagnostic(&logs);
    assert_null_result_slices(&messages, &system, &spec).await;
    assert_no_block_is_null(&system, &spec, &broken).await;
}

fn assert_skipping_diagnostic(logs: &Logs) {
    let diagnostics =
        logs.matching("context_management polyfill: skipping edits due to error: broken");
    assert_eq!(diagnostics.len(), 1, "{:?}", logs.lines());
    assert!(
        diagnostics[0].starts_with("ERROR "),
        "the diagnostic is exception-level: {}",
        diagnostics[0]
    );
}

async fn assert_null_result_slices(messages: &[Json], system: &Json, spec: &Json) {
    let null = ScriptedDispatch::new(Ok(None));
    let result = prepare_context_managed_request(
        context(messages, Some(system), Some(spec), None, None),
        &null,
    )
    .await
    .expect("a null result is not an error");
    assert_eq!(result, Some(history_result()));
}

async fn assert_no_block_is_null(system: &Json, spec: &Json, dispatcher: &ScriptedDispatch) {
    let plain = decided(PLAIN);
    let result = prepare_context_managed_request(
        context(&plain, Some(system), Some(spec), None, None),
        dispatcher,
    )
    .await
    .expect("nothing to slice");
    assert!(result.is_none());
}

#[tokio::test]
async fn noncompact_failure_surfaces() {
    let (messages, system, _tools) = history_fixture();
    let clear = parse(r#"{"edits":[{"type":"clear_tool_uses_20250919"}]}"#);
    let mixed =
        parse(r#"{"edits":[{"type":"clear_tool_uses_20250919"},{"type":"compact_20260112"}]}"#);
    let unknown = parse(r#"{"edits":[{"type":"compact_20260112"},{"type":"mystery_edit"}]}"#);
    assert_surfaced_failure(&messages, &system, &clear).await;
    assert_surfaced_failure(&messages, &system, &mixed).await;
    assert_surfaced_failure(&messages, &system, &unknown).await;
    assert_trigger_refusal(&messages, &system).await;
}

async fn assert_surfaced_failure(messages: &[Json], system: &Json, spec: &Json) {
    let dispatcher = ScriptedDispatch::new(Err(DispatchFailure::Other(String::from("broken"))));
    let (logs, _guard) = capture_logs();
    let error = prepare_context_managed_request(
        context(messages, Some(system), Some(spec), None, None),
        &dispatcher,
    )
    .await
    .expect_err("a failed non-compact edit surfaces");
    let failure = context_failure(error);
    assert_eq!(failure.status_code, 500);
    assert_eq!(
        failure.message,
        "context_management polyfill failed: broken"
    );
    assert_eq!(
        logs.matching("context_management polyfill: skipping edits due to error: broken")
            .len(),
        1
    );
}

async fn assert_trigger_refusal(messages: &[Json], system: &Json) {
    let low = parse(LOW_TRIGGER);
    let never = NeverCalled;
    let counter = RecordingHistoryCounter::new(50_001);
    let registered = editors(None, &counter, &never);
    let (logs, _guard) = capture_logs();
    let error = prepare_context_managed_request(
        context(messages, Some(system), Some(&low), None, None),
        &registered,
    )
    .await
    .expect_err("the trigger refusal propagates");
    let failure = context_failure(error);
    assert_eq!(failure.status_code, 400);
    assert_eq!(
        failure.message,
        "context_management.compact_20260112.trigger.value must be at least 50000 tokens"
    );
    assert!(
        logs.matching("skipping edits").is_empty(),
        "a client error is not a swallowed failure: {:?}",
        logs.lines()
    );
}
