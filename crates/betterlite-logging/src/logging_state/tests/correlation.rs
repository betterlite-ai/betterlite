use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{build_in, init, params, redactor, text};
use crate::logging_state::LoggingInit;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, restore_correlation_context,
    restore_correlation_guarded, sanitize_correlation_id,
};

enum Scenario {
    Sanitize(&'static str),
    SanitizeBound,
    Unguarded,
    GuardedOwn,
    GuardedForeignTrace,
    GuardedForeignSession,
    Isolated,
}

fn enabled() -> CorrelationSettings {
    CorrelationSettings {
        supports_correlation_logging: true,
        request_correlation_in_logs: true,
    }
}

fn correlated() -> LoggingInit {
    LoggingInit {
        litellm_trace_id: Some("inner-trace".to_owned()),
        kwargs: params(&[("litellm_session_id", text("inner-session"))]),
        ..init()
    }
}

fn context(trace_id: &str, session_id: &str) -> CorrelationContext {
    CorrelationContext {
        trace_id: trace_id.to_owned(),
        session_id: session_id.to_owned(),
    }
}

fn original_context() -> CorrelationContext {
    context("outer-trace", "outer-session")
}

fn restore_outcome(mut working: CorrelationContext, guarded: bool) -> String {
    let mut original = original_context();
    let state = build_in(correlated(), &mut original, enabled());
    if guarded {
        restore_correlation_guarded(&state, &mut working);
    } else {
        restore_correlation_context(&state, &mut working);
    }
    format!("{}/{}", working.trace_id, working.session_id)
}

fn bound_outcome() -> String {
    let long = "a".repeat(300);
    let bounded = sanitize_correlation_id(&long, &redactor());
    format!(
        "{}:{}",
        bounded.len(),
        bounded.chars().all(|character| character == 'a')
    )
}

fn isolated_outcome() -> String {
    let mut first_context = context("outer-a", "outer-a");
    let mut second_context = context("outer-b", "outer-b");
    let mut first = build_in(
        LoggingInit {
            litellm_trace_id: Some("inner-a".to_owned()),
            litellm_call_id: "call-a".to_owned(),
            ..init()
        },
        &mut first_context,
        enabled(),
    );
    let second = build_in(
        LoggingInit {
            litellm_trace_id: Some("inner-b".to_owned()),
            litellm_call_id: "call-b".to_owned(),
            ..init()
        },
        &mut second_context,
        enabled(),
    );
    first
        .model_call_details
        .insert("only-first".to_owned(), Json::Bool(true));
    first
        .litellm_params
        .insert("only-first".to_owned(), text("yes"));
    format!(
        "{}/{}/{}/{}",
        second.litellm_trace_id,
        second.litellm_call_id,
        second.model_call_details.contains_key("only-first"),
        second.litellm_params.contains_key("only-first")
    )
}

fn outcome(scenario: &Scenario) -> String {
    match scenario {
        Scenario::Sanitize(value) => sanitize_correlation_id(value, &redactor()),
        Scenario::SanitizeBound => bound_outcome(),
        Scenario::Unguarded => restore_outcome(context("foreign", "foreign"), false),
        Scenario::GuardedOwn => restore_outcome(context("inner-trace", "inner-session"), true),
        Scenario::GuardedForeignTrace => restore_outcome(context("foreign", "inner-session"), true),
        Scenario::GuardedForeignSession => restore_outcome(context("inner-trace", "foreign"), true),
        Scenario::Isolated => isolated_outcome(),
    }
}

#[rstest]
#[case::a_plain_id_is_kept(Scenario::Sanitize("trace-1"), "trace-1")]
#[case::control_characters_are_stripped(Scenario::Sanitize("tra\rce\u{0}-1"), "trace-1")]
#[case::a_secret_shaped_id_is_redacted(
    Scenario::Sanitize("sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    "REDACTED"
)]
#[case::a_non_breaking_space_is_not_printable(Scenario::Sanitize("sp\u{a0}ace"), "space")]
#[case::printable_unicode_is_kept(Scenario::Sanitize("\u{e9}t\u{e9}"), "\u{e9}t\u{e9}")]
#[case::a_long_id_is_bounded(Scenario::SanitizeBound, "256:true")]
#[case::an_unguarded_restore_always_wins(Scenario::Unguarded, "outer-trace/outer-session")]
#[case::a_guarded_restore_runs_when_the_ids_are_ours(
    Scenario::GuardedOwn,
    "outer-trace/outer-session"
)]
#[case::a_guarded_restore_skips_a_foreign_trace_id(
    Scenario::GuardedForeignTrace,
    "foreign/inner-session"
)]
#[case::a_guarded_restore_skips_a_foreign_session_id(
    Scenario::GuardedForeignSession,
    "inner-trace/foreign"
)]
#[case::each_request_owns_its_state(Scenario::Isolated, "inner-b/call-b/false/false")]
fn logging_state_behavior_tests_behavior(#[case] scenario: Scenario, #[case] expected: &str) {
    assert_eq!(outcome(&scenario), expected);
}
