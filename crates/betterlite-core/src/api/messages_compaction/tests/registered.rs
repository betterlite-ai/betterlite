use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::summary_model_access::Settings;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_models::decisions::Json;

use crate::api::messages_compaction::dispatch::{DispatchRequest, apply_context_management};
use crate::api::messages_compaction::editor::RegisteredEditors;
use crate::api::messages_context_clear::constants::{
    CLEAR_TOOL_USES_EDIT_TYPE, CLEARED_TOOL_RESULT_CONTENT, COMPACT_EDIT_TYPE,
};
use crate::api::messages_context_clear::result::{CompactionKind, PolyfillResult};
use crate::api::summary_execution::completion::{ExecutorCompletion, SummaryCompletion};

use super::support::{
    FixedClearCounter, FixedHistoryCounter, NoBudgets, NoChecks, NoHooks, NoRows, decided,
    deployment, executor, field, iteration, parse, text_field,
};

const SUMMARY_BODY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":0,"model":"gpt-4o-mini","choices":[{"index":0,"message":{"role":"assistant","content":"<summary>S</summary>"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}}"#;
const COMPACT_HISTORY: &str = r#"[{"role":"user","content":"hello"},
    {"role":"assistant","content":"hi"},{"role":"user","content":"question"}]"#;
const TOOL_HISTORY: &str = r#"[
    {"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_0","content":"first result"}]},
    {"role":"assistant","content":[{"type":"tool_use","id":"tool_0","name":"lookup","input":{}}]},
    {"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_1","content":"second result"}]},
    {"role":"assistant","content":[{"type":"tool_use","id":"tool_1","name":"lookup","input":{}}]},
    {"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_2","content":"third result"}]},
    {"role":"assistant","content":[{"type":"tool_use","id":"tool_2","name":"lookup","input":{}}]},
    {"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_3","content":"fourth result"}]},
    {"role":"assistant","content":[{"type":"tool_use","id":"tool_3","name":"lookup","input":{}}]},
    {"role":"user","content":"final question"}
]"#;

struct SummaryRun {
    result: PolyfillResult,
    request: Request,
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

fn registered<'services>(
    settings: &'services Settings,
    counter: &'services FixedHistoryCounter,
    completion: &'services dyn SummaryCompletion,
    clear_count: usize,
) -> RegisteredEditors<'services, NoChecks, NoRows, NoBudgets, NoHooks, FixedHistoryCounter> {
    RegisteredEditors {
        settings: Some(settings),
        proxy: None,
        budgets: None,
        hooks: None,
        history_counter: counter,
        clear_counter: Arc::new(FixedClearCounter { count: clear_count }),
        direct: completion,
    }
}

fn dispatch<'spec>(
    messages: Vec<Json>,
    system: Option<Json>,
    spec: &'spec Json,
) -> DispatchRequest<'spec> {
    DispatchRequest {
        model: "main-model",
        messages,
        tools: None,
        system,
        context_management_spec: Some(spec),
        litellm_metadata: None,
        llm_router: None,
        user_api_key_auth: None,
    }
}

async fn run_registered(
    messages: &str,
    system: Option<Json>,
    spec: &Json,
    clear_count: usize,
) -> SummaryRun {
    let serving = serve_summary();
    let executor = executor();
    let summary_deployment = deployment(&format!("http://{}/v1", serving.address));
    let completion = ExecutorCompletion {
        executor: &executor,
        deployment: &summary_deployment,
    };
    let settings = summary_settings();
    let counter = FixedHistoryCounter {
        conversation: 50_001,
        text: 0,
    };
    let editors = registered(&settings, &counter, &completion, clear_count);
    let result = apply_context_management(dispatch(decided(messages), system, spec), &editors)
        .await
        .expect("the registered editors run");
    let request = serving.finish().expect("one provider request");
    SummaryRun { result, request }
}

fn max_tokens(body: &Json) -> u64 {
    match field(body, "max_tokens") {
        Some(Json::Number(number)) => number.as_u64().expect("max_tokens is an integer"),
        other => panic!("the summary request carries max_tokens: {other:?}"),
    }
}

fn assert_one_summary_request(request: &Request) {
    assert_eq!(request.line, "POST /v1/chat/completions HTTP/1.1");
    let body = parse(&request.body);
    assert_eq!(text_field(&body, "model"), Some("gpt-4o-mini"));
    assert_eq!(max_tokens(&body), 4096);
}

fn assert_compaction_result(result: &PolyfillResult) {
    let block = result
        .compaction_block
        .as_ref()
        .expect("a compaction block");
    assert_eq!(block.kind, CompactionKind::Compaction);
    assert_eq!(block.content.as_deref(), Some("S"));
    assert_eq!(result.applied_edits.len(), 1);
    assert_eq!(
        result.applied_edits[0].edit_type.as_deref(),
        Some(COMPACT_EDIT_TYPE)
    );
    assert_eq!(result.applied_edits[0].summary_input_tokens, Some(12));
    assert_eq!(result.applied_edits[0].summary_output_tokens, Some(3));
    assert_eq!(result.iterations_usage, Some(vec![iteration(12, 3)]));
}

fn assert_clear_then_compact(result: &PolyfillResult) {
    assert_eq!(result.applied_edits.len(), 2);
    assert_eq!(
        result.applied_edits[0].edit_type.as_deref(),
        Some(CLEAR_TOOL_USES_EDIT_TYPE)
    );
    assert_eq!(result.applied_edits[0].cleared_tool_uses, Some(1));
    assert_eq!(
        result.applied_edits[1].edit_type.as_deref(),
        Some(COMPACT_EDIT_TYPE)
    );
    let block = result
        .compaction_block
        .as_ref()
        .expect("a compaction block");
    assert_eq!(block.content.as_deref(), Some("S"));
}

#[tokio::test]
async fn registered_summary_editor_http() {
    let spec = parse(
        r#"{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":50000}}]}"#,
    );
    let run = run_registered(COMPACT_HISTORY, None, &spec, 0).await;
    assert_one_summary_request(&run.request);
    assert_compaction_result(&run.result);
}

#[tokio::test]
async fn registered_summary_editor_http_carries_the_system_prompt() {
    let spec = parse(
        r#"{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":50000}}]}"#,
    );
    let system = Some(parse(r#""BASE SYSTEM""#));
    let run = run_registered(COMPACT_HISTORY, system, &spec, 0).await;
    assert_one_summary_request(&run.request);
    assert!(
        run.request.body.contains("BASE SYSTEM"),
        "the system prompt becomes the summary call's first message: {}",
        run.request.body
    );
    assert_compaction_result(&run.result);
}

#[tokio::test]
async fn registered_summary_editor_http_clears_before_compacting() {
    let spec = parse(
        r#"{"edits":[{"type":"clear_tool_uses_20250919",
            "trigger":{"type":"input_tokens","value":50000}},
            {"type":"compact_20260112","trigger":{"type":"input_tokens","value":50000}}]}"#,
    );
    let run = run_registered(TOOL_HISTORY, None, &spec, 50_001).await;
    assert_eq!(run.request.line, "POST /v1/chat/completions HTTP/1.1");
    assert!(
        run.request.body.contains(CLEARED_TOOL_RESULT_CONTENT),
        "the cleared tool result reaches the summary request: {}",
        run.request.body
    );
    assert!(
        run.request.body.contains("tool_call_id"),
        "the tool history reaches the summary call in OpenAI shape: {}",
        run.request.body
    );
    assert_clear_then_compact(&run.result);
}
