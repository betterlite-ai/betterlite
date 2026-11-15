use std::time::Duration;

use betterlite_context::agentic_callback::{AgenticCall, AgenticGate, AgenticLogging};
use betterlite_context::agentic_loop_plan::AgenticLoopPlan;
use rstest::rstest;

use crate::compresr_recovery::error::RecoveryError;
use crate::compresr_recovery::plan::should_run;
use crate::compresr_recovery::store::{ORIGINALS_TTL, RecoveryStore};
use crate::compresr_recovery::tools::{holds_retrieve_tool, merged_tools};
use crate::generic_guardrail_contracts::GuardrailToolParam;

use super::support::{
    HASH_ONE, NOW, RETRIEVE_TOOL, Setup, anthropic_response, chat_response, map, messages,
    plan_with, raw, responses_response, seeded, setup, value,
};

const OTHER_CALL_ONLY: &str = r#"{"choices":[{"message":{"content":"x","tool_calls":[{"id":"c","type":"function","function":{"name":"search","arguments":"{}"}}]}}]}"#;
const SEARCH_TOOL: &str = r#"[{"type":"function","function":{"name":"search"}}]"#;

fn gate(response: &str, tools: Option<&str>) -> AgenticGate {
    let parsed = tools.map(messages);
    let body = raw(response);
    let kwargs = map("{}");
    let call = AgenticCall {
        response: &body,
        model: "gpt-4o",
        messages: &[],
        tools: parsed.as_deref(),
        stream: false,
        custom_llm_provider: "openai",
        kwargs: &kwargs,
    };
    should_run(&call).unwrap()
}

fn store_with_original() -> RecoveryStore {
    seeded(&[(HASH_ONE, "original one")])
}

fn assert_skipped(plan: &AgenticLoopPlan) {
    assert!(!plan.run_agentic_loop);
    assert!(plan.request_patch.is_none());
    assert!(plan.metadata.is_empty());
}

#[rstest]
#[case::chat(chat_response, "call_1")]
#[case::anthropic(anthropic_response, "toolu_1")]
#[case::responses(responses_response, "call_1")]
fn gate_runs_for_a_retrieve_call_and_keeps_only_it_agentic(
    #[case] build: fn(&str) -> String,
    #[case] id: &str,
) {
    let outcome = gate(&build(HASH_ONE), Some(RETRIEVE_TOOL));
    assert!(outcome.run);
    let expected = format!(
        r#"{{"tool_calls":[{{"id":"{id}","type":"function","name":"compresr_retrieve","arguments":{{"hash":"{HASH_ONE}"}}}}]}}"#
    );
    assert_eq!(serde_json::to_string(&outcome.tools).unwrap(), expected);
}

#[rstest]
#[case::no_tools(None)]
#[case::empty_tools(Some("[]"))]
#[case::other_tool_only(Some(SEARCH_TOOL))]
fn gate_skips_a_request_without_the_retrieve_tool_agentic(#[case] tools: Option<&str>) {
    let outcome = gate(&chat_response(HASH_ONE), tools);
    assert!(!outcome.run);
    assert!(outcome.tools.is_empty());
}

#[test]
fn gate_accepts_the_anthropic_tool_shape_agentic() {
    let tools = r#"[{"name":"compresr_retrieve","input_schema":{"type":"object"}}]"#;
    assert!(gate(&chat_response(HASH_ONE), Some(tools)).run);
}

#[rstest]
#[case::other_call_only(OTHER_CALL_ONLY)]
#[case::no_calls(r#"{"choices":[{"message":{"content":"hi"}}]}"#)]
fn gate_skips_a_response_without_a_retrieve_call_agentic(#[case] response: &str) {
    let outcome = gate(response, Some(RETRIEVE_TOOL));
    assert!(!outcome.run);
    assert!(outcome.tools.is_empty());
}

#[test]
fn gate_rejects_a_response_that_is_not_an_object_agentic() {
    let body = raw(r#""just text""#);
    let kwargs = map("{}");
    let tools = messages(RETRIEVE_TOOL);
    let call = AgenticCall {
        response: &body,
        model: "gpt-4o",
        messages: &[],
        tools: Some(&tools),
        stream: false,
        custom_llm_provider: "openai",
        kwargs: &kwargs,
    };
    assert!(matches!(
        should_run(&call),
        Err(RecoveryError::Response { .. })
    ));
}

#[test]
fn plan_names_the_tool_type_and_keeps_the_thread_agentic() {
    let plan = plan_with(&store_with_original(), &chat_response(HASH_ONE), &setup()).unwrap();
    assert!(plan.run_agentic_loop);
    assert!(!plan.terminate);
    assert!(plan.response_override.is_none());
    assert!(plan.stop_reason.is_none());
    assert_eq!(
        serde_json::to_string(&plan.metadata).unwrap(),
        r#"{"tool_type":"compresr_retrieve"}"#
    );
    let patch = plan.request_patch.unwrap();
    assert!(patch.tools.is_none());
    assert_eq!(patch.messages.unwrap().len(), 3);
}

#[test]
fn plan_without_stored_originals_skips_the_loop_agentic() {
    let plan = plan_with(
        &RecoveryStore::default(),
        &chat_response(HASH_ONE),
        &setup(),
    )
    .unwrap();
    assert_skipped(&plan);
}

#[rstest]
#[case::no_logging(None)]
#[case::no_call_id(Some(None))]
#[case::empty_call_id(Some(Some("")))]
fn plan_without_a_call_id_skips_the_loop_agentic(#[case] call_id: Option<Option<&str>>) {
    let mut request = setup();
    request.logging = call_id.map(|id| AgenticLogging {
        call_id: id.map(str::to_owned),
        agentic_loop_params: None,
    });
    let plan = plan_with(&store_with_original(), &chat_response(HASH_ONE), &request).unwrap();
    assert_skipped(&plan);
}

#[rstest]
#[case::other_caller("scope-b", NOW)]
#[case::no_scope("", NOW)]
#[case::expired("scope-a", NOW.saturating_add(ORIGINALS_TTL))]
fn plan_for_another_caller_or_after_expiry_skips_the_loop_agentic(
    #[case] scope: &'static str,
    #[case] now: Duration,
) {
    let mut request = setup();
    request.scope = scope;
    request.now = now;
    let plan = plan_with(&store_with_original(), &chat_response(HASH_ONE), &request).unwrap();
    assert_skipped(&plan);
}

#[test]
fn plan_prunes_expired_entries_before_retrieving_agentic() {
    let store = store_with_original();
    let mut request = setup();
    request.now = NOW.saturating_add(ORIGINALS_TTL);
    plan_with(&store, &chat_response(HASH_ONE), &request).unwrap();
    assert!(store.state.lock().unwrap().entries.is_empty());
}

#[rstest]
#[case::no_params(None, "gpt-4o")]
#[case::from_params(Some(r#"{"model":"gpt-4o-2025"}"#), "gpt-4o-2025")]
#[case::empty_model(Some(r#"{"model":""}"#), "gpt-4o")]
#[case::model_not_text(Some(r#"{"model":5}"#), "gpt-4o")]
#[case::no_model_key(Some("{}"), "gpt-4o")]
fn plan_model_comes_from_the_logging_params_agentic(
    #[case] params: Option<&str>,
    #[case] expected: &str,
) {
    let mut request = setup();
    request.logging = Some(AgenticLogging {
        call_id: Some("call-1".to_owned()),
        agentic_loop_params: params.map(map),
    });
    let plan = plan_with(&store_with_original(), &chat_response(HASH_ONE), &request).unwrap();
    assert_eq!(plan.request_patch.unwrap().model.as_deref(), Some(expected));
}

#[rstest]
#[case::neither("{}", "{}", None)]
#[case::optional_params(r#"{"max_tokens":100}"#, "{}", Some(100))]
#[case::kwargs("{}", r#"{"max_tokens":50}"#, Some(50))]
#[case::optional_params_win(r#"{"max_tokens":100}"#, r#"{"max_tokens":50}"#, Some(100))]
#[case::null_falls_through(r#"{"max_tokens":null}"#, r#"{"max_tokens":50}"#, Some(50))]
fn plan_max_tokens_prefers_the_optional_params_agentic(
    #[case] optional: &str,
    #[case] kwargs: &str,
    #[case] expected: Option<u64>,
) {
    let mut request = setup();
    request.optional_params = map(optional);
    request.kwargs = map(kwargs);
    let plan = plan_with(&store_with_original(), &chat_response(HASH_ONE), &request).unwrap();
    assert_eq!(plan.request_patch.unwrap().max_tokens, expected);
}

#[rstest]
#[case::text_in_optional_params(r#"{"max_tokens":"lots"}"#, "{}")]
#[case::negative_in_kwargs("{}", r#"{"max_tokens":-1}"#)]
fn plan_rejects_a_max_tokens_that_is_not_an_integer_agentic(
    #[case] optional: &str,
    #[case] kwargs: &str,
) {
    let mut request = setup();
    request.optional_params = map(optional);
    request.kwargs = map(kwargs);
    let outcome = plan_with(&store_with_original(), &chat_response(HASH_ONE), &request);
    assert!(matches!(
        outcome,
        Err(RecoveryError::FieldType {
            field: "max_tokens",
            ..
        })
    ));
}

#[test]
fn plan_forwards_the_other_params_and_cleans_the_kwargs_agentic() {
    let mut request = setup();
    request.optional_params = map(r#"{"temperature":0.2,"max_tokens":9}"#);
    request.kwargs =
        map(r#"{"litellm_call_id":"call-1","_compresr_state":1,"litellm_logging_obj":"x"}"#);
    let plan = plan_with(&store_with_original(), &chat_response(HASH_ONE), &request).unwrap();
    let patch = plan.request_patch.unwrap();
    assert_eq!(
        serde_json::to_string(&patch.optional_params).unwrap(),
        r#"{"temperature":0.2}"#
    );
    assert_eq!(
        serde_json::to_string(&patch.kwargs).unwrap(),
        r#"{"litellm_call_id":"call-1"}"#
    );
}

#[rstest]
#[case::tool_calls_not_a_list(r#"{"tool_calls":5}"#, true)]
#[case::no_tool_calls("{}", false)]
fn plan_reads_the_gate_tool_calls_agentic(#[case] tools: &str, #[case] fails: bool) {
    let mut request: Setup = setup();
    request.tools = Some(map(tools));
    let outcome = plan_with(&store_with_original(), &chat_response(HASH_ONE), &request);
    if fails {
        assert!(matches!(outcome, Err(RecoveryError::ToolCalls { .. })));
    } else {
        assert_skipped(&outcome.unwrap());
    }
}

#[rstest]
#[case::no_tools(None, 1)]
#[case::without_it(Some(SEARCH_TOOL), 2)]
#[case::already_there(Some(RETRIEVE_TOOL), 1)]
fn merged_tools_append_the_retrieve_tool_once_agentic(
    #[case] existing: Option<&str>,
    #[case] count: usize,
) {
    let parsed: Option<Vec<GuardrailToolParam>> =
        existing.map(|json| serde_json::from_str(json).unwrap());
    let merged = merged_tools(parsed.as_deref()).unwrap();
    assert_eq!(merged.len(), count);
    assert!(holds_retrieve_tool(&merged));
}

#[test]
fn merged_tools_define_the_retrieve_tool_like_litellm_agentic() {
    let merged = merged_tools(None).unwrap();
    let expected = r#"{"type":"function","function":{"name":"compresr_retrieve","description":"Retrieve the original, uncompressed content behind a Compresr compression marker. Call this when a compression marker's hash points at content you need in full.","parameters":{"type":"object","properties":{"hash":{"type":"string","description":"The 24-character hex hash from the compression marker."}},"required":["hash"]}}}"#;
    let encoded = serde_json::to_string(&merged[0]).unwrap();
    assert_eq!(value(&encoded), value(expected));
}
