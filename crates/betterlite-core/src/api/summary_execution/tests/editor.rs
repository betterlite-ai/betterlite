use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::summary_model_access::Settings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use serde::Serialize;

use crate::api::messages_context_clear::constants::{
    COMPACT_DEFAULT_INSTRUCTIONS, COMPACT_EDIT_TYPE, COMPACT_NO_TOOL_CALLS_SUFFIX,
    COMPACT_SUMMARY_SYSTEM_PREFIX,
};
use crate::api::messages_context_clear::result::{
    CompactionKind, PolyfillResult, UsageIteration, UsageKind,
};
use crate::api::summary_execution::error::SummaryExecutionError;
use crate::api::summary_execution::extract::ExtractionError;
use crate::api::summary_history::last_question::CONTINUE_PROMPT;

use super::support::{
    Answer, Counted, Fixture, Port, Sample, capture_logs, list, object, principal, run, spec,
};

const MODEL: &str = "main-model";
const TAIL: &str =
    r#"[{"role":"assistant","content":"ack"},{"role":"user","content":"recent question"}]"#;
const BUDGETED: &str = r#"{
    "models": ["other-model"],
    "user_id": "u1",
    "user_model_max_budget": {"sum": {"budget_limit": 1.0, "time_period": "1d"}}
}"#;

fn augmented(summary: &str) -> String {
    format!("{COMPACT_SUMMARY_SYSTEM_PREFIX}{summary}\n\nBASE SYSTEM")
}

fn encoded<Value>(value: &Value) -> String
where
    Value: Serialize,
{
    serde_json::to_string(value).expect("the value encodes")
}

fn answer(raw: &str) -> Answer {
    Answer::Raw(serde_json::from_str(raw).expect("a completion answer"))
}

fn assert_error(result: &PolyfillResult, code: &str) {
    let edit = result.applied_edits.first().expect("one applied edit");
    assert_eq!(edit.edit_type.as_deref(), Some(COMPACT_EDIT_TYPE));
    assert_eq!(edit.error.as_deref(), Some(code));
}

fn assert_untouched_tail(result: &PolyfillResult) {
    assert_eq!(encoded(&result.messages), TAIL);
    assert_eq!(result.system, Some(DecisionJson::Text(augmented("S"))));
    assert!(result.compaction_block.is_none());
    assert!(result.iterations_usage.is_none());
}

#[tokio::test]
async fn trigger_validation_precedes_every_call() {
    let fixture = Fixture::default();
    let (logs, _guard) = capture_logs();
    let mut sample = Sample::default();
    sample.spec =
        spec(r#"{"type":"compact_20260112","trigger":{"type":"input_tokens","value":49999}}"#);
    match run(&fixture, sample.request(MODEL), Port::Routed).await {
        Err(SummaryExecutionError::Trigger(trigger)) => {
            assert_eq!(trigger.status_code, 400);
            assert_eq!(
                trigger.message,
                "context_management.compact_20260112.trigger.value must be at least 50000 tokens"
            );
        }
        other => panic!("expected the trigger error: {other:?}"),
    }
    assert!(fixture.counter.calls().is_empty());
    assert!(fixture.recorder.steps().is_empty());
    assert_eq!(fixture.summary_calls(), 0);
    assert!(!logs.contains("compaction trigger"));
}

#[tokio::test]
async fn absent_trigger_logs_the_default_threshold() {
    let fixture = Fixture::default();
    let (logs, _guard) = capture_logs();
    let mut sample = Sample::default();
    sample.spec = spec(r#"{"type":"compact_20260112"}"#);
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert!(logs.contains(
        "INFO compact_20260112: request has compaction trigger (input_tokens threshold=150000)"
    ));
    assert_eq!(
        result
            .applied_edits
            .first()
            .expect("one applied edit")
            .edit_type
            .as_deref(),
        Some(COMPACT_EDIT_TYPE)
    );
}

#[tokio::test]
async fn pause_after_compaction_only_warns() {
    let mut sample = Sample::default();
    sample.spec = spec(
        r#"{"type":"compact_20260112","trigger":{"type":"output_tokens","value":60000},
            "pause_after_compaction":true}"#,
    );
    let fixture = Fixture::default();
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    let edit = result.applied_edits.first().expect("one applied edit");
    assert_eq!(
        edit.warnings.clone().expect("warnings"),
        [
            "unsupported_trigger_type_output_tokens_using_input_tokens",
            "pause_after_compaction_ignored"
        ]
    );
    assert!(result.compaction_block.is_some());
    sample.spec = spec(r#"{"type":"compact_20260112","pause_after_compaction":false}"#);
    let quiet = Fixture::default();
    let result = run(&quiet, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert!(
        result
            .applied_edits
            .first()
            .expect("one applied edit")
            .warnings
            .is_none()
    );
}

#[tokio::test]
async fn missing_summary_model_slices_without_counting() {
    let mut fixture = Fixture::default();
    fixture.settings = None;
    let result = run(&fixture, Sample::default().request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_model_not_configured");
    assert_eq!(encoded(&result.messages), TAIL);
    assert_eq!(result.system, Some(DecisionJson::Text(augmented("S"))));
    assert!(fixture.counter.calls().is_empty());
    assert_eq!(fixture.summary_calls(), 0);
    let mut fixture = Fixture::default();
    fixture.settings = Some(Settings::default());
    let result = run(&fixture, Sample::default().request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_model_not_configured");
    assert_eq!(encoded(&result.messages), TAIL);
}

#[tokio::test]
async fn slice_only_falls_back_to_the_last_question_only_when_empty() {
    let mut fixture = Fixture::default();
    fixture.settings = Some(Settings::default());
    let mut sample = Sample::default();
    sample.system = None;
    sample.messages =
        list(r#"[{"role":"assistant","content":[{"type":"compaction","content":"S"}]}]"#);
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_model_not_configured");
    let expected = format!(r#"[{{"role":"user","content":"{CONTINUE_PROMPT}"}}]"#);
    assert_eq!(encoded(&result.messages), expected);
    assert_eq!(
        result.system,
        Some(DecisionJson::Text(
            "Previous conversation summary: S".to_owned()
        ))
    );
}

#[tokio::test]
async fn count_at_the_threshold_stops_before_the_summary() {
    let (logs, _guard) = capture_logs();
    let mut fixture = Fixture::default();
    fixture.counter.conversation = 150_000;
    let mut sample = Sample::default();
    sample.spec = spec(r#"{"type":"compact_20260112"}"#);
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert!(
        result
            .applied_edits
            .first()
            .expect("one applied edit")
            .error
            .is_none()
    );
    assert_eq!(fixture.summary_calls(), 0);
    assert_eq!(encoded(&result.messages), TAIL);
    assert_eq!(result.system, Some(DecisionJson::Text(augmented("S"))));
    assert!(logs.contains("DEBUG compact_20260112: current_tokens=150000 trigger=150000"));
}

#[tokio::test]
async fn count_above_the_threshold_proceeds_to_the_summary() {
    let (logs, _guard) = capture_logs();
    let mut fixture = Fixture::default();
    fixture.counter.conversation = 150_001;
    let mut sample = Sample::default();
    sample.spec = spec(r#"{"type":"compact_20260112"}"#);
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_eq!(fixture.summary_calls(), 1);
    assert!(result.compaction_block.is_some());
    assert!(logs.contains("DEBUG compact_20260112: current_tokens=150001 trigger=150000"));
}

#[tokio::test]
async fn count_reads_the_stripped_history_and_the_augmented_system() {
    let mut fixture = Fixture::default();
    fixture.counter.conversation = 150_000;
    fixture.counter.text = 1;
    let mut sample = Sample::default();
    sample.spec = spec(r#"{"type":"compact_20260112"}"#);
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_eq!(fixture.summary_calls(), 1);
    let calls = fixture.counter.calls();
    let Counted::Conversation {
        model,
        messages,
        tools,
    } = calls.first().expect("one conversation count")
    else {
        panic!("expected a conversation count");
    };
    assert_eq!(model.as_str(), MODEL);
    assert_eq!(encoded(messages), TAIL);
    assert!(tools.is_none());
    assert_eq!(fixture.counter.texts(), [augmented("S")]);
    assert!(result.compaction_block.is_some());
}

#[tokio::test]
async fn counter_failure_warns_and_continues() {
    let (logs, _guard) = capture_logs();
    let mut fixture = Fixture::default();
    fixture.counter.fail = true;
    let result = run(&fixture, Sample::default().request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_eq!(fixture.summary_calls(), 0);
    assert_untouched_tail(&result);
    assert!(logs.contains(
        "WARN compact_20260112: token_counter failed; assuming under threshold: either text or messages must be provided"
    ));
    assert!(logs.contains("DEBUG compact_20260112: current_tokens=0 trigger=50000"));
}

#[tokio::test]
async fn access_denial_skips_budget_rate_and_the_summary() {
    let (logs, _guard) = capture_logs();
    let mut fixture = Fixture::default();
    fixture.deny_access();
    let mut sample = Sample::default();
    sample.principal = Some(principal(r#"{"models":["other-model"]}"#));
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_model_access_denied");
    assert_eq!(fixture.recorder.steps(), ["access"]);
    assert_eq!(fixture.summary_calls(), 0);
    assert_untouched_tail(&result);
    assert!(logs.contains(
        "WARN compact_20260112: caller not authorized for summary_model=sum; skipping summary call"
    ));
}

#[tokio::test]
async fn a_denied_summary_keeps_an_empty_tail_empty() {
    let mut fixture = Fixture::default();
    fixture.deny_access();
    let mut sample = Sample::default();
    sample.principal = Some(principal(r#"{"models":["other-model"]}"#));
    sample.messages =
        list(r#"[{"role":"assistant","content":[{"type":"compaction","content":"S"}]}]"#);
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_model_access_denied");
    assert!(result.messages.is_empty());
    assert_eq!(fixture.summary_calls(), 0);
}

#[tokio::test]
async fn budget_denial_runs_after_access_and_stops_before_rate() {
    let (logs, _guard) = capture_logs();
    let mut fixture = Fixture::default();
    fixture.deny_budget();
    let mut sample = Sample::default();
    sample.principal = Some(principal(BUDGETED));
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_model_budget_exceeded");
    assert_eq!(fixture.recorder.steps(), ["access", "budget"]);
    assert_eq!(fixture.summary_calls(), 0);
    assert_untouched_tail(&result);
    assert!(logs.contains(
        "WARN compact_20260112: caller over model budget for summary_model=sum; skipping summary call"
    ));
}

#[tokio::test]
async fn rate_denial_runs_after_budget() {
    let (logs, _guard) = capture_logs();
    let mut fixture = Fixture::default();
    fixture.deny_rate();
    let mut sample = Sample::default();
    sample.principal = Some(principal(BUDGETED));
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_model_rate_limit_exceeded");
    assert_eq!(fixture.recorder.steps(), ["access", "budget", "rate"]);
    assert_eq!(fixture.summary_calls(), 0);
    assert_untouched_tail(&result);
    assert!(logs.contains(
        "WARN compact_20260112: caller over rate limit for summary_model=sum; skipping summary call"
    ));
}

#[tokio::test]
async fn an_allowed_policy_runs_every_check_then_one_summary() {
    let fixture = Fixture::default();
    let mut sample = Sample::default();
    sample.principal = Some(principal(BUDGETED));
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_eq!(fixture.recorder.steps(), ["access", "budget", "rate"]);
    assert_eq!(fixture.router.count(), 1);
    assert_eq!(fixture.direct.count(), 0);
    assert!(result.compaction_block.is_some());
}

#[tokio::test]
async fn an_available_router_is_called_once() {
    let fixture = Fixture::default();
    run(&fixture, Sample::default().request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_eq!(fixture.router.count(), 1);
    assert_eq!(fixture.direct.count(), 0);
}

#[tokio::test]
async fn without_a_router_the_direct_pipeline_is_called_once() {
    let fixture = Fixture::default();
    run(
        &fixture,
        Sample::default().request(MODEL),
        Port::Direct(&fixture.direct),
    )
    .await
    .expect("the editor runs");
    assert_eq!(fixture.router.count(), 0);
    assert_eq!(fixture.direct.count(), 1);
}

#[tokio::test]
async fn summary_request_arguments_match_litellm() {
    let fixture = Fixture::default();
    let mut sample = Sample::default();
    sample.metadata = Some(object(
        r#"{"user_api_key":"k","litellm_call_id":"c","user_api_key_end_user_id":"end-1",
            "provider_metadata":{"x":1}}"#,
    ));
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    let call = fixture
        .router
        .calls()
        .first()
        .expect("one summary call")
        .clone();
    assert_eq!(call.model, "sum");
    assert_eq!(call.max_tokens, 4096);
    assert_eq!(call.timeout_seconds, 60.0);
    assert_eq!(call.user.as_deref(), Some("end-1"));
    assert_eq!(call.region, None);
    let keys: Vec<&str> = call.metadata.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "user_api_key",
            "user_api_key_end_user_id",
            "litellm_call_id"
        ]
    );
    assert_eq!(
        call.metadata.get("user_api_key").and_then(Json::as_str),
        Some("k")
    );
    assert!(call.metadata.get("provider_metadata").is_none());
    let messages = &call.messages;
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0].get("role").and_then(Json::as_str),
        Some("system")
    );
    assert_eq!(
        messages[0].get("content").and_then(Json::as_str),
        Some(augmented("S").as_str())
    );
    assert_eq!(
        serde_json::to_string(&messages[1]).expect("the turn encodes"),
        r#"{"role":"assistant","content":"ack"}"#
    );
    let expected = format!(
        r#"{{"role":"user","content":"recent question\n\n{COMPACT_DEFAULT_INSTRUCTIONS}"}}"#
    );
    assert_eq!(
        serde_json::to_string(&messages[2]).expect("the question encodes"),
        expected
    );
    assert!(
        !serde_json::to_string(messages)
            .expect("the call encodes")
            .contains("Do not call any tools")
    );
    assert!(result.compaction_block.is_some());
}

#[tokio::test]
async fn declared_tools_add_the_no_tools_suffix() {
    let fixture = Fixture::default();
    let mut sample = Sample::default();
    sample.tools = Some(list(
        r#"[{"name":"lookup","description":"d","input_schema":{"type":"object"}}]"#,
    ));
    run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    let call = fixture
        .router
        .calls()
        .first()
        .expect("one summary call")
        .clone();
    let prompt = serde_json::to_string(call.messages.last().expect("the prompt turn"))
        .expect("the prompt encodes");
    assert!(prompt.ends_with(&format!("{COMPACT_NO_TOOL_CALLS_SUFFIX}\"}}")));
}

#[tokio::test]
async fn configured_max_tokens_travel_to_the_call() {
    let mut fixture = Fixture::default();
    fixture.settings = Some(Settings {
        context_management_summary_model: Some(String::from("sum")),
        context_management_summary_max_tokens: 512,
    });
    run(&fixture, Sample::default().request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_eq!(
        fixture
            .router
            .calls()
            .first()
            .expect("one summary call")
            .max_tokens,
        512
    );
}

#[rstest]
#[case::end_user(r#"{"user_api_key_end_user_id":"end-1"}"#, Some("end-1"))]
#[case::no_end_user(r#"{"user_api_key":"k"}"#, None)]
#[case::empty_end_user(r#"{"user_api_key_end_user_id":""}"#, None)]
#[case::number_end_user(r#"{"user_api_key_end_user_id":7}"#, None)]
#[tokio::test]
async fn only_a_nonempty_string_end_user_travels(
    #[case] metadata: &str,
    #[case] expected: Option<&str>,
) {
    let fixture = Fixture::default();
    let mut sample = Sample::default();
    sample.metadata = Some(object(metadata));
    run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_eq!(
        fixture
            .router
            .calls()
            .first()
            .expect("one summary call")
            .user
            .as_deref(),
        expected
    );
}

#[tokio::test]
async fn the_principal_region_travels_to_the_route() {
    let mut fixture = Fixture::default();
    fixture.attach_policy = false;
    let mut sample = Sample::default();
    sample.principal = Some(principal(r#"{"allowed_model_region":"us"}"#));
    run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    let call = fixture
        .router
        .calls()
        .first()
        .expect("one summary call")
        .clone();
    assert_eq!(call.region.as_deref(), Some("us"));
    assert_eq!(call.user, None);
}

#[test]
fn an_unknown_model_region_never_reaches_the_editor() {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(r#"{"allowed_model_region":"apac"}"#).expect("principal fields");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect_err("the region is typed");
}

#[tokio::test]
async fn summary_success_replaces_the_old_prefix() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::default();
    let result = run(&fixture, Sample::default().request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    let call = fixture
        .router
        .calls()
        .first()
        .expect("one summary call")
        .clone();
    assert!(
        serde_json::to_string(&call.messages)
            .expect("the call encodes")
            .contains("Previous conversation summary: S")
    );
    assert_eq!(result.system, Some(DecisionJson::Text(augmented("NEW"))));
    assert_eq!(
        serde_json::to_string(&result.messages).expect("the result encodes"),
        r#"[{"role":"user","content":"recent question"}]"#
    );
    let block = result.compaction_block.clone().expect("a compaction block");
    assert_eq!(block.kind, CompactionKind::Compaction);
    assert_eq!(block.content.as_deref(), Some("NEW"));
    assert_eq!(block.signature, None);
    let edit = result.applied_edits.first().expect("one applied edit");
    assert_eq!(edit.summary_input_tokens, Some(12));
    assert_eq!(edit.summary_output_tokens, Some(3));
    assert!(edit.error.is_none());
    assert_eq!(
        result.iterations_usage.clone().expect("iterations usage"),
        [UsageIteration {
            kind: UsageKind::Compaction,
            input_tokens: Some(12),
            output_tokens: Some(3),
        }]
    );
    assert!(logs.contains(
        "INFO compact_20260112: compaction summary added to main call system prefix (3 chars)"
    ));
    assert_eq!(fixture.summary_calls(), 1);
}

#[tokio::test]
async fn the_summary_keeps_only_the_last_real_user_question() {
    let fixture = Fixture::default();
    let mut sample = Sample::default();
    sample.messages = list(
        r#"[{"role":"user","content":"q1"},{"role":"assistant","content":"a1"},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"t"}]},
            {"role":"assistant","content":"a2"}]"#,
    );
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_eq!(
        serde_json::to_string(&result.messages).expect("the result encodes"),
        r#"[{"role":"user","content":"q1"}]"#
    );
}

#[tokio::test]
async fn without_an_eligible_question_the_continuation_prompt_is_kept() {
    let fixture = Fixture::default();
    let mut sample = Sample::default();
    sample.messages = list(r#"[{"role":"assistant","content":"a1"}]"#);
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    let expected = format!(r#"[{{"role":"user","content":"{CONTINUE_PROMPT}"}}]"#);
    assert_eq!(
        serde_json::to_string(&result.messages).expect("the result encodes"),
        expected
    );
}

#[tokio::test]
async fn a_failed_summary_call_preserves_the_tail() {
    let (logs, _guard) = capture_logs();
    let mut fixture = Fixture::default();
    fixture.router.answer = Answer::Failure(String::from("boom"));
    let result = run(&fixture, Sample::default().request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_call_failed");
    assert_untouched_tail(&result);
    assert!(
        result
            .applied_edits
            .first()
            .expect("one applied edit")
            .summary_input_tokens
            .is_none()
    );
    assert!(logs.contains("WARN compact_20260112: summary call failed: boom"));
}

#[tokio::test]
async fn a_failed_summary_keeps_an_empty_tail_empty() {
    let mut fixture = Fixture::default();
    fixture.router.answer = Answer::Failure(String::from("boom"));
    let mut sample = Sample::default();
    sample.messages =
        list(r#"[{"role":"assistant","content":[{"type":"compaction","content":"S"}]}]"#);
    let result = run(&fixture, sample.request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_call_failed");
    assert!(result.messages.is_empty());
}

#[tokio::test]
async fn a_response_without_a_summary_block_fails_extraction_quietly() {
    let (logs, _guard) = capture_logs();
    let mut fixture = Fixture::default();
    fixture.router.answer = answer(
        r#"{"choices":[{"message":{"content":"no tag"}}],"usage":{"prompt_tokens":1,"completion_tokens":2}}"#,
    );
    let result = run(&fixture, Sample::default().request(MODEL), Port::Routed)
        .await
        .expect("the editor runs");
    assert_error(&result, "summary_extraction_failed");
    assert_untouched_tail(&result);
    let edit = result.applied_edits.first().expect("one applied edit");
    assert_eq!(edit.summary_input_tokens, None);
    assert_eq!(edit.summary_output_tokens, None);
    assert!(!logs.contains("summary call failed"));
}

#[tokio::test]
async fn a_nonstring_text_part_is_an_error() {
    let mut fixture = Fixture::default();
    fixture.router.answer =
        answer(r#"{"choices":[{"message":{"content":[{"type":"text","text":5}]}}]}"#);
    match run(&fixture, Sample::default().request(MODEL), Port::Routed).await {
        Err(SummaryExecutionError::Extraction(ExtractionError::NonTextPart)) => {}
        other => panic!("expected the non-text part error: {other:?}"),
    }
}
