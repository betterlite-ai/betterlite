#![cfg(test)]

mod harness;

use betterlite_config::sections::guardrail_params::base::OnViolation;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use self::harness::{
    Fixture, Outcome, PROMPT_PREFIX, Predicate, SendLog, Sockets, Split, TestFailure,
    blocked_message, blocking_guardrail, capture, check_data, closes, frames, guardrail_input,
    json, lock, one_block, selection,
};
use super::callback::{GuardrailViolation, RealtimeGuardrail};
use super::error::RealtimeGuardrailsError;
use super::message::DEFAULT_BLOCK_MESSAGE;
use super::runner::{RealtimeGuardrails, RunRealtimeGuardrails};

const TOOL_RESULT: &str = r#"{"type":"conversation.item.create","item":{"type":"function_call_output","call_id":"call-1","output":"safe"}}"#;
const CANCEL: &str = r#"{"type":"response.cancel"}"#;
const SPOKEN: &str = r#"{"type":"conversation.item.create","item":{"type":"message","role":"user","content":[{"type":"input_text","text":"Say exactly the following message to the user, word for word, do not add anything else: denied"}]}}"#;
const CREATE: &str = r#"{"type":"response.create"}"#;
const NOTICE: &str = r#"{"type":"error","error":{"type":"guardrail_violation","message":"denied","code":"content_policy_violation"}}"#;

#[tokio::test]
async fn realtime_guardrail_selection_and_clean_result() {
    let default = selection(None).await;
    assert!(!default.blocked);
    assert_eq!(default.count, 0);
    assert_eq!(default.frames, 0);
    assert_eq!(
        default.checks,
        vec![(
            GuardrailEventHooks::RealtimeInputTranscription,
            check_data()
        )]
    );
    assert_eq!(default.applied, vec![guardrail_input()]);
    assert_eq!(default.skipped, 0);
    assert_eq!(default.false_checks, 1);

    let empty = selection(Some(&[])).await;
    assert!(!empty.blocked);
    assert!(empty.checks.is_empty() && empty.applied.is_empty() && empty.skipped == 0);
    assert_eq!(empty.false_checks, 0);

    let pre_call = selection(Some(&[GuardrailEventHooks::PreCall])).await;
    assert!(!pre_call.blocked);
    assert_eq!(
        pre_call.checks,
        vec![(GuardrailEventHooks::PreCall, check_data())]
    );
    assert_eq!(pre_call.applied, vec![guardrail_input()]);
    assert_eq!(pre_call.skipped, 0);
    assert_eq!(pre_call.false_checks, 1);
}

#[tokio::test]
async fn realtime_guardrail_clean_apply_continues() {
    let fixture = Fixture::new();
    let mut first = blocking_guardrail();
    first.outcome = Outcome::Clean;
    let mut second = blocking_guardrail();
    second.outcome = Outcome::Clean;
    let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![&first, &second];
    let mut state = fixture.state();
    let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
        state: &mut state,
        provider: None,
        guardrails: &guardrails,
    };
    let blocked = runner
        .run_realtime_guardrails("hello", None, None, None)
        .await
        .expect("a clean run returns");
    assert!(!blocked);
    assert_eq!(runner.state.setup.violation_count, 0);
    assert_eq!(first.records().1.len(), 1);
    assert_eq!(second.records().1.len(), 1);
    assert!(fixture.backend.sent().is_empty() && fixture.client.sent().is_empty());
    assert!(fixture.timeline.entries().is_empty());
}

#[tokio::test]
async fn realtime_guardrail_block_sequence() {
    let fixture = Fixture::new();
    let blocking = blocking_guardrail();
    let later = blocking_guardrail();
    let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![&blocking, &later];
    let mut state = fixture.state();
    let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
        state: &mut state,
        provider: None,
        guardrails: &guardrails,
    };
    let blocked = runner
        .run_realtime_guardrails("hello", Some("item-1"), Some(TOOL_RESULT), None)
        .await
        .expect("the block completes");
    assert!(blocked);
    assert_eq!(runner.state.setup.violation_count, 1);
    assert_eq!(
        fixture.timeline.entries(),
        vec!["backend", "backend", "client", "backend", "backend"]
    );
    assert_eq!(
        frames(&fixture.backend),
        vec![json(TOOL_RESULT), json(CANCEL), json(SPOKEN), json(CREATE)]
    );
    assert_eq!(frames(&fixture.client), vec![json(NOTICE)]);
    assert!(later.records().0.is_empty() && later.records().1.is_empty());
    assert_eq!(closes(&fixture), (0, 0));
}

#[rstest]
#[case::configured_message(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Text(String::from("detail-denied")))])),
        message: String::from("raised-denied"),
    },
    Some("custom"),
    Json::Text(String::from("custom")),
    "custom"
)]
#[case::empty_override_uses_truthy_detail_error(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Text(String::from("detail-denied")))])),
        message: String::from("raised-denied"),
    },
    Some(""),
    Json::Text(String::from("detail-denied")),
    "detail-denied"
)]
#[case::null_detail_error_falls_back_to_the_exception(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Null)])),
        message: String::from("raised-denied"),
    },
    Some(""),
    Json::Text(String::from("raised-denied")),
    "raised-denied"
)]
#[case::false_detail_error_is_falsy(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Bool(false))])),
        message: String::from("raised-denied"),
    },
    Some(""),
    Json::Text(String::from("raised-denied")),
    "raised-denied"
)]
#[case::zero_detail_error_is_falsy(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Number(Number::from(0_u8)))])),
        message: String::from("raised-denied"),
    },
    Some(""),
    Json::Text(String::from("raised-denied")),
    "raised-denied"
)]
#[case::huge_unsigned_detail_error_is_truthy(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Number(Number::from(u64::MAX)))])),
        message: String::from("raised-denied"),
    },
    None,
    Json::Number(Number::from(u64::MAX)),
    "18446744073709551615"
)]
#[case::float_zero_detail_error_is_falsy(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Number(Number::from_f64(0.0).expect("zero is finite")))])),
        message: String::from("raised-denied"),
    },
    Some(""),
    Json::Text(String::from("raised-denied")),
    "raised-denied"
)]
#[case::empty_text_detail_error_is_falsy(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Text(String::new()))])),
        message: String::from("raised-denied"),
    },
    Some(""),
    Json::Text(String::from("raised-denied")),
    "raised-denied"
)]
#[case::empty_list_detail_error_is_falsy(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::List(Vec::new()))])),
        message: String::from("raised-denied"),
    },
    Some(""),
    Json::Text(String::from("raised-denied")),
    "raised-denied"
)]
#[case::empty_object_detail_error_is_falsy(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Object(IndexMap::new()))])),
        message: String::from("raised-denied"),
    },
    Some(""),
    Json::Text(String::from("raised-denied")),
    "raised-denied"
)]
#[case::empty_exception_uses_the_default_message(
    GuardrailViolation {
        detail: Json::Null,
        message: String::new(),
    },
    None,
    Json::Text(String::from(DEFAULT_BLOCK_MESSAGE)),
    DEFAULT_BLOCK_MESSAGE
)]
#[case::text_detail_is_used_as_is(
    GuardrailViolation {
        detail: Json::Text(String::from("plain detail")),
        message: String::from("raised-denied"),
    },
    None,
    Json::Text(String::from("plain detail")),
    "plain detail"
)]
#[case::number_detail_is_stringified(
    GuardrailViolation {
        detail: Json::Number(Number::from(7_u8)),
        message: String::from("raised-denied"),
    },
    None,
    Json::Text(String::from("7")),
    "7"
)]
#[case::nonstring_detail_error_is_kept(
    GuardrailViolation {
        detail: Json::Object(IndexMap::from([("error".to_owned(), Json::Number(Number::from(42_u8)))])),
        message: String::from("raised-denied"),
    },
    None,
    Json::Number(Number::from(42_u8)),
    "42"
)]
#[tokio::test]
async fn realtime_guardrail_message_precedence(
    #[case] violation: GuardrailViolation,
    #[case] configured: Option<&str>,
    #[case] expected: Json,
    #[case] prompt: &str,
) {
    let blocked = blocked_message(violation, configured).await;
    assert_eq!(blocked.message, expected);
    assert_eq!(blocked.prompt, format!("{PROMPT_PREFIX}{prompt}"));
}

#[tokio::test]
async fn realtime_guardrail_message_precedence_unexpected_error() {
    let (logs, _guard) = capture();
    let fixture = Fixture::new();
    let mut guardrail = blocking_guardrail();
    guardrail.outcome = Outcome::Unexpected;
    guardrail.violation = GuardrailViolation {
        detail: Json::Object(IndexMap::from([(
            "error".to_owned(),
            Json::Text(String::from("looks like a block")),
        )])),
        message: String::from("boom"),
    };
    let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![&guardrail];
    let mut state = fixture.state();
    let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
        state: &mut state,
        provider: None,
        guardrails: &guardrails,
    };
    let error = runner
        .run_realtime_guardrails("hello", None, None, None)
        .await
        .expect_err("the unexpected error is rethrown");
    assert!(matches!(
        error,
        RealtimeGuardrailsError::Unexpected(TestFailure::Unexpected)
    ));
    assert_eq!(runner.state.setup.violation_count, 0);
    assert!(fixture.backend.sent().is_empty() && fixture.client.sent().is_empty());
    assert!(fixture.timeline.entries().is_empty());
    assert!(logs.messages().iter().any(|message| message
        == "[realtime guardrail] unexpected error in apply_guardrail: the scripted guardrail raised an unexpected error"));
}

#[rstest]
#[case::on_violation(Some(OnViolation::EndSession), None)]
#[case::zero_threshold(None, Some(0))]
#[case::negative_threshold(None, Some(-1))]
#[case::no_threshold(None, None)]
#[tokio::test]
async fn realtime_guardrail_end_session(
    #[case] on_violation: Option<OnViolation>,
    #[case] threshold: Option<i64>,
) {
    let (logs, _guard) = capture();
    let fixture = Fixture::new();
    let mut guardrail = blocking_guardrail();
    guardrail.policy.on_violation = on_violation;
    guardrail.policy.end_session_after_n_fails = threshold;
    assert!(
        one_block(&fixture, &guardrail, "hello")
            .await
            .expect("the block completes")
    );
    let closes_backend =
        on_violation == Some(OnViolation::EndSession) || threshold.is_some_and(|limit| limit <= 1);
    let expected_closes = if closes_backend { 1 } else { 0 };
    let expected_timeline = if closes_backend {
        vec!["backend", "client", "backend", "backend", "backend-close"]
    } else {
        vec!["backend", "client", "backend", "backend"]
    };
    assert_eq!(fixture.timeline.entries(), expected_timeline);
    assert_eq!(closes(&fixture), (expected_closes, 0));
    let ending = logs
        .messages()
        .iter()
        .any(|message| message == "[realtime guardrail] ending session after violation 1");
    assert_eq!(ending, closes_backend);
}

#[tokio::test]
async fn realtime_guardrail_end_session_threshold_two() {
    let (logs, _guard) = capture();
    let fixture = Fixture::new();
    let mut guardrail = blocking_guardrail();
    guardrail.policy.end_session_after_n_fails = Some(2);
    let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![&guardrail];
    let mut state = fixture.state();
    let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
        state: &mut state,
        provider: None,
        guardrails: &guardrails,
    };
    assert!(
        runner
            .run_realtime_guardrails("hello", None, None, None)
            .await
            .expect("the first block completes")
    );
    assert_eq!(runner.state.setup.violation_count, 1);
    assert_eq!(closes(&fixture), (0, 0));
    assert!(
        runner
            .run_realtime_guardrails("hello", None, None, None)
            .await
            .expect("the second block completes")
    );
    assert_eq!(runner.state.setup.violation_count, 2);
    assert_eq!(closes(&fixture), (1, 0));
    assert_eq!(fixture.timeline.entries().last(), Some(&"backend-close"));
    let messages = logs.messages();
    assert!(
        messages
            .iter()
            .any(|message| message == "[realtime guardrail] ending session after violation 2")
    );
    assert!(messages.iter().any(|message| {
        message.starts_with("[realtime guardrail] BLOCKED transcript (violation 1):")
    }));
    assert!(messages.iter().any(|message| {
        message.starts_with("[realtime guardrail] BLOCKED transcript (violation 2):")
    }));
}

#[tokio::test]
async fn realtime_guardrail_logs_only_the_first_eighty_transcript_characters() {
    let (logs, _guard) = capture();
    let fixture = Fixture::new();
    let guardrail = blocking_guardrail();
    let transcript = "\u{e9}".repeat(100);
    assert!(
        one_block(&fixture, &guardrail, &transcript)
            .await
            .expect("the block completes")
    );
    let prefix: String = transcript.chars().take(80).collect();
    let blocked = logs
        .messages()
        .into_iter()
        .find(|message| message.contains("BLOCKED transcript"))
        .expect("the block is logged");
    assert_eq!(
        blocked,
        format!("[realtime guardrail] BLOCKED transcript (violation 1): {prefix:?}")
    );
}

#[tokio::test]
async fn realtime_guardrail_send_failure_leaves_the_count_unchanged() {
    let mut fixture = Fixture::new();
    fixture.backend = SendLog::failing(vec![1]);
    let guardrail = blocking_guardrail();
    let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![&guardrail];
    let mut state = fixture.state();
    let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
        state: &mut state,
        provider: None,
        guardrails: &guardrails,
    };
    let error = runner
        .run_realtime_guardrails("hello", None, None, None)
        .await
        .expect_err("the spoken-message send fails");
    assert!(matches!(error, RealtimeGuardrailsError::Send(_)));
    assert_eq!(runner.state.setup.violation_count, 0);
    assert_eq!(fixture.timeline.entries(), vec!["backend", "client"]);
    assert_eq!(fixture.backend.sent().len(), 1);
    assert_eq!(fixture.client.sent().len(), 1);
    assert_eq!(closes(&fixture), (0, 0));
}

#[tokio::test]
async fn realtime_guardrail_client_failure_is_returned_before_the_count() {
    let mut fixture = Fixture::new();
    fixture.client = SendLog::failing(vec![0]);
    let guardrail = blocking_guardrail();
    let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![&guardrail];
    let mut state = fixture.state();
    let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
        state: &mut state,
        provider: None,
        guardrails: &guardrails,
    };
    let error = runner
        .run_realtime_guardrails("hello", None, None, None)
        .await
        .expect_err("the client notice fails");
    assert!(matches!(error, RealtimeGuardrailsError::Client(_)));
    assert_eq!(runner.state.setup.violation_count, 0);
    assert_eq!(fixture.timeline.entries(), vec!["backend"]);
    assert_eq!(fixture.backend.sent().len(), 1);
    assert_eq!(closes(&fixture), (0, 0));
}

#[tokio::test]
async fn realtime_guardrail_predicate_failure_is_returned() {
    let fixture = Fixture::new();
    let mut guardrail = blocking_guardrail();
    guardrail.predicate = Predicate::Fails;
    let error = one_block(&fixture, &guardrail, "hello")
        .await
        .expect_err("the predicate fails");
    assert!(matches!(
        error,
        RealtimeGuardrailsError::Predicate(TestFailure::Predicate)
    ));
    assert!(fixture.timeline.entries().is_empty());
    assert!(fixture.backend.sent().is_empty() && fixture.client.sent().is_empty());
}

#[tokio::test]
async fn realtime_guardrail_close_failure_is_returned_after_the_count() {
    let (logs, _guard) = capture();
    let mut fixture = Fixture::new();
    fixture.close_failure = true;
    let mut guardrail = blocking_guardrail();
    guardrail.policy.on_violation = Some(OnViolation::EndSession);
    let error = one_block(&fixture, &guardrail, "hello")
        .await
        .expect_err("the backend close fails");
    assert!(matches!(error, RealtimeGuardrailsError::Close(_)));
    assert_eq!(fixture.backend.sent().len(), 3);
    assert!(
        logs.messages()
            .iter()
            .any(|message| message == "[realtime guardrail] ending session after violation 1")
    );
    assert!(
        !logs
            .messages()
            .iter()
            .any(|message| message.contains("BLOCKED transcript"))
    );
}

#[tokio::test]
async fn realtime_guardrail_missing_client_is_an_error() {
    let mut fixture = Fixture::new();
    fixture.sockets = Sockets::BackendOnly;
    let guardrail = blocking_guardrail();
    let error = one_block(&fixture, &guardrail, "hello")
        .await
        .expect_err("the client notice has no socket");
    assert!(matches!(error, RealtimeGuardrailsError::ClientMissing));
    assert_eq!(fixture.timeline.entries(), vec!["backend"]);
    assert_eq!(fixture.backend.sent().len(), 1);
}

#[tokio::test]
async fn realtime_guardrail_missing_backend_close_is_an_error() {
    let (logs, _guard) = capture();
    let mut fixture = Fixture::new();
    fixture.sockets = Sockets::ClientOnly;
    let mut guardrail = blocking_guardrail();
    guardrail.policy.on_violation = Some(OnViolation::EndSession);
    let provider = Split::default();
    let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![&guardrail];
    let mut state = fixture.state();
    let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
        state: &mut state,
        provider: Some(&provider),
        guardrails: &guardrails,
    };
    let error = runner
        .run_realtime_guardrails("hello", None, None, None)
        .await
        .expect_err("the close has no backend");
    assert!(matches!(error, RealtimeGuardrailsError::BackendMissing));
    assert_eq!(runner.state.setup.violation_count, 1);
    assert_eq!(lock(&provider.seen).len(), 3);
    assert_eq!(fixture.client.sent().len(), 1);
    assert!(
        !logs
            .messages()
            .iter()
            .any(|message| message.contains("BLOCKED transcript"))
    );
}

#[tokio::test]
async fn realtime_guardrail_count_survives_the_run() {
    let fixture = Fixture::new();
    let guardrail = blocking_guardrail();
    assert!(
        one_block(&fixture, &guardrail, "hello")
            .await
            .expect("the block completes")
    );
    let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![&guardrail];
    let mut state = fixture.state();
    state.setup.violation_count = 5;
    let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
        state: &mut state,
        provider: None,
        guardrails: &guardrails,
    };
    assert!(
        runner
            .run_realtime_guardrails("hello", None, None, None)
            .await
            .expect("the block completes")
    );
    assert_eq!(runner.state.setup.violation_count, 6);
}
