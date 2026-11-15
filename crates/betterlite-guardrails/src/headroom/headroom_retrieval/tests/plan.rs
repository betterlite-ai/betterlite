use std::time::Duration;

use betterlite_context::agentic_callback::AgenticLogging;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, AgenticLoopRequestPatch, JsonMap};

use super::support;

const CHAT: &str = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"toolu_1","type":"function","function":{"name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}}]}}]}"#;
const UNREACHABLE: &str = "http://127.0.0.1:1";

fn patch(plan: &AgenticLoopPlan) -> AgenticLoopRequestPatch {
    plan.request_patch.clone().expect("a request patch")
}

fn logging(call_id: Option<&str>, params: Option<&JsonMap>) -> AgenticLogging {
    AgenticLogging {
        call_id: call_id.map(str::to_owned),
        agentic_loop_params: params.cloned(),
    }
}

fn rejected() -> String {
    r#"{"role":"tool","tool_call_id":"toolu_1","content":"[Headroom: hash=abcdef123456 was not produced by the current request]"}"#.to_owned()
}

fn retrieved() -> String {
    r#"{"role":"tool","tool_call_id":"toolu_1","content":"original text"}"#.to_owned()
}

async fn seeded_server() -> (support::Server, support::Harness) {
    let server = support::start(vec![
        support::compress_reply(&[support::HASH]),
        support::retrieve_reply("original text"),
    ])
    .await;
    let harness = support::harness(&server.base);
    harness.seed(support::CALL_ID, &[support::HASH]).await;
    (server, harness)
}

#[tokio::test]
async fn the_plan_runs_the_loop_and_labels_the_tool_type() {
    let (_server, harness) = seeded_server().await;
    let call = support::loop_call(CHAT);
    let plan = support::build_plan(&harness, &call).await;
    assert!(plan.run_agentic_loop);
    assert!(plan.response_override.is_none());
    assert!(!plan.terminate);
    assert!(plan.stop_reason.is_none());
    assert_eq!(
        plan.metadata.get("tool_type").map(|raw| raw.get()),
        Some("\"headroom_ccr\"")
    );
    assert!(patch(&plan).tools.is_none());
}

#[tokio::test]
async fn the_retrieval_request_reaches_the_service() {
    let (server, harness) = seeded_server().await;
    let call = support::loop_call(CHAT);
    support::build_plan(&harness, &call).await;
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/v1/compress");
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[1].path, "/v1/retrieve/abcdef123456");
    assert_eq!(requests[1].authorization.as_deref(), Some("Bearer sk-test"));
}

#[tokio::test]
async fn the_query_parameter_reaches_the_retrieval() {
    let (server, harness) = seeded_server().await;
    let mut call = support::loop_call(CHAT);
    call.tools = support::gate(&[support::tool_call(
        "toolu_1",
        r#"{"hash":"abcdef123456","query":"find it"}"#,
    )]);
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(messages[2], support::json(&retrieved()));
    assert_eq!(
        server.requests()[1].path,
        "/v1/retrieve/abcdef123456?query=find+it"
    );
}

#[tokio::test]
async fn an_uppercase_hash_is_lowercased_for_validation() {
    let (server, harness) = seeded_server().await;
    let mut call = support::loop_call(CHAT);
    call.tools = support::gate(&[support::tool_call("toolu_1", r#"{"hash":"ABCDEF123456"}"#)]);
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(messages[2], support::json(&retrieved()));
    assert_eq!(server.requests()[1].path, "/v1/retrieve/abcdef123456");
}

#[tokio::test]
async fn a_hash_this_request_did_not_issue_is_rejected() {
    let harness = support::harness(UNREACHABLE);
    let call = support::loop_call(CHAT);
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(messages[2], support::json(&rejected()));
}

#[tokio::test]
async fn a_hash_issued_for_another_call_is_rejected() {
    let server = support::start(vec![support::compress_reply(&[support::HASH])]).await;
    let harness = support::harness(&server.base);
    harness.seed("call-2", &[support::HASH]).await;
    let call = support::loop_call(CHAT);
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(messages[2], support::json(&rejected()));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn an_expired_issuer_is_rejected() {
    let (server, harness) = seeded_server().await;
    harness.set_now(Duration::from_secs(901));
    let call = support::loop_call(CHAT);
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(messages[2], support::json(&rejected()));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn the_kwargs_call_id_selects_the_issued_set() {
    let server = support::start(vec![
        support::compress_reply(&[support::HASH]),
        support::retrieve_reply("original text"),
    ])
    .await;
    let harness = support::harness(&server.base);
    harness.seed("call-2", &[support::HASH]).await;
    let mut call = support::loop_call(CHAT);
    call.kwargs = support::json_map(&[("litellm_call_id", "\"call-2\"")]);
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(messages[2], support::json(&retrieved()));
}

#[tokio::test]
async fn the_logging_call_id_wins_over_the_kwargs() {
    let (server, harness) = seeded_server().await;
    let mut call = support::loop_call(CHAT);
    call.kwargs = support::json_map(&[("litellm_call_id", "\"kwargs-call\"")]);
    call.logging = Some(logging(Some(support::CALL_ID), None));
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(messages[2], support::json(&retrieved()));
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn an_empty_logging_call_id_falls_back_to_the_kwargs() {
    let (_server, harness) = seeded_server().await;
    let mut call = support::loop_call(CHAT);
    call.logging = Some(logging(Some(""), None));
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(messages[2], support::json(&retrieved()));
}

#[tokio::test]
async fn a_missing_call_id_rejects_every_hash() {
    let server = support::start(vec![support::compress_reply(&[support::HASH])]).await;
    let harness = support::harness(&server.base);
    harness.seed(support::CALL_ID, &[support::HASH]).await;
    let mut call = support::loop_call(CHAT);
    call.kwargs = JsonMap::new();
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(messages[2], support::json(&rejected()));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn the_model_comes_from_the_logging_parameters() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    let params = support::json_map(&[("model", "\"gpt-4o-2025\"")]);
    call.logging = Some(logging(Some(support::CALL_ID), Some(&params)));
    let plan = support::build_plan(&harness, &call).await;
    assert_eq!(patch(&plan).model.as_deref(), Some("gpt-4o-2025"));
}

#[tokio::test]
async fn an_empty_logging_model_falls_back_to_the_request_model() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    let params = support::json_map(&[("model", "\"\"")]);
    call.logging = Some(logging(Some(support::CALL_ID), Some(&params)));
    let plan = support::build_plan(&harness, &call).await;
    assert_eq!(patch(&plan).model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn a_non_text_logging_model_falls_back() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    let params = support::json_map(&[("model", "5")]);
    call.logging = Some(logging(Some(support::CALL_ID), Some(&params)));
    let plan = support::build_plan(&harness, &call).await;
    assert_eq!(patch(&plan).model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn the_request_model_is_the_fallback() {
    let harness = support::harness(UNREACHABLE);
    let call = support::loop_call(CHAT);
    let plan = support::build_plan(&harness, &call).await;
    assert_eq!(patch(&plan).model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn max_tokens_comes_from_the_optional_parameters() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    call.optional_params = support::json_map(&[("max_tokens", "100")]);
    call.kwargs = support::json_map(&[("litellm_call_id", "\"call-1\""), ("max_tokens", "200")]);
    let plan = support::build_plan(&harness, &call).await;
    let fields = patch(&plan);
    assert_eq!(fields.max_tokens, Some(100));
    assert!(!fields.optional_params.contains_key("max_tokens"));
    assert_eq!(
        support::values(&fields.kwargs),
        support::json(r#"{"litellm_call_id":"call-1","max_tokens":200}"#)
    );
}

#[tokio::test]
async fn max_tokens_falls_back_to_the_kwargs() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    call.kwargs = support::json_map(&[("litellm_call_id", "\"call-1\""), ("max_tokens", "200")]);
    let plan = support::build_plan(&harness, &call).await;
    assert_eq!(patch(&plan).max_tokens, Some(200));
}

#[tokio::test]
async fn an_explicit_zero_max_tokens_is_kept() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    call.optional_params = support::json_map(&[("max_tokens", "0")]);
    call.kwargs = support::json_map(&[("litellm_call_id", "\"call-1\""), ("max_tokens", "200")]);
    let plan = support::build_plan(&harness, &call).await;
    assert_eq!(patch(&plan).max_tokens, Some(0));
}

#[tokio::test]
async fn a_null_max_tokens_falls_back_to_the_kwargs() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    call.optional_params = support::json_map(&[("max_tokens", "null")]);
    call.kwargs = support::json_map(&[("litellm_call_id", "\"call-1\""), ("max_tokens", "200")]);
    let plan = support::build_plan(&harness, &call).await;
    assert_eq!(patch(&plan).max_tokens, Some(200));
}

#[tokio::test]
async fn a_non_integer_max_tokens_is_refused() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    call.optional_params = support::json_map(&[("max_tokens", "\"100\"")]);
    match support::plan_error(&harness, &call).await {
        AgenticHookError::Failed { message, .. } => {
            assert_eq!(message, "`max_tokens` must be an integer");
        }
    }
}

#[tokio::test]
async fn the_optional_parameters_lose_only_max_tokens() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    call.optional_params = support::json_map(&[
        ("temperature", "0.2"),
        ("max_tokens", "100"),
        ("top_p", "1"),
    ]);
    let plan = support::build_plan(&harness, &call).await;
    assert_eq!(
        support::values(&patch(&plan).optional_params),
        support::json(r#"{"temperature":0.2,"top_p":1}"#)
    );
}

#[tokio::test]
async fn the_kwargs_lose_the_headroom_and_logging_keys() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    call.kwargs = support::json_map(&[
        ("litellm_call_id", "\"call-1\""),
        ("stream", "false"),
        ("max_tokens", "200"),
        ("_headroom_interception_converted_stream", "true"),
        ("_headroom", "1"),
        ("litellm_logging_obj", r#"{"x":1}"#),
        ("headroom_note", "\"kept\""),
    ]);
    let plan = support::build_plan(&harness, &call).await;
    assert_eq!(
        support::values(&patch(&plan).kwargs),
        support::json(
            r#"{"litellm_call_id":"call-1","stream":false,"max_tokens":200,"headroom_note":"kept"}"#
        )
    );
}

#[tokio::test]
async fn a_plan_without_gate_tools_has_no_retrieval() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    call.tools = JsonMap::new();
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(
        messages,
        vec![
            support::json(r#"{"role":"user","content":"hello"}"#),
            support::json(r#"{"role":"assistant","content":null,"tool_calls":[]}"#),
        ]
    );
}

#[tokio::test]
async fn malformed_gate_tools_are_a_hook_error() {
    let harness = support::harness(UNREACHABLE);
    let mut call = support::loop_call(CHAT);
    call.tools = support::json_map(&[("tool_calls", "5")]);
    match support::plan_error(&harness, &call).await {
        AgenticHookError::Failed { callback, message } => {
            assert_eq!(callback, "headroom");
            assert_eq!(
                message,
                "the agentic loop tools do not hold the tool_calls list the gate produced"
            );
        }
    }
}

#[tokio::test]
async fn an_unreadable_response_is_a_plan_failure() {
    let harness = support::harness(UNREACHABLE);
    let call = support::loop_call("[1]");
    match support::plan_error(&harness, &call).await {
        AgenticHookError::Failed { message, .. } => {
            assert_eq!(
                message,
                "the model response is not a chat, Responses or Anthropic response object"
            );
        }
    }
}

#[tokio::test]
async fn a_404_content_reaches_the_tool_message() {
    let server = support::start(vec![
        support::compress_reply(&[support::HASH]),
        support::reply(404, "gone"),
    ])
    .await;
    let harness = support::harness(&server.base);
    harness.seed(support::CALL_ID, &[support::HASH]).await;
    let call = support::loop_call(CHAT);
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(
        messages[2],
        support::json(
            r#"{"role":"tool","tool_call_id":"toolu_1","content":"[Headroom: hash=abcdef123456 not found or expired]"}"#
        )
    );
}
