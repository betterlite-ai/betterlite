use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::routes::CallType;
use betterlite_logging::prometheus_latency::guardrail_series::GuardrailStatus;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{Answer, CLASS, Fake, Harness, Observed, auth, fake, guardrail, harness, map};
use super::{Class, NAME, context, http, metadata_field, shaped, strings};
use crate::guardrails::callback_caps::caps::{CallbackCapabilities, GuardrailCapabilities};
use crate::guardrails::callback_caps::errors::{GuardrailGateError, ProxyHookError};
use crate::guardrails::callback_caps::traits::{GuardrailEvent, ProxyCallback};
use crate::guardrails::hook_execution::during_call::during_call_hook;

fn capabilities(callbacks: Vec<Fake>) -> CallbackCapabilities {
    let mut listed: Vec<Arc<dyn ProxyCallback>> = Vec::new();
    for callback in callbacks {
        listed.push(Arc::new(callback));
    }
    CallbackCapabilities::from_callbacks(&listed)
}

async fn run(
    harness: &Harness,
    capabilities: &CallbackCapabilities,
    user_api_key_dict: Option<&UserAPIKeyAuth>,
    call_type: CallType,
) -> Result<(), ProxyHookError> {
    let mut data = map("{}");
    let scope = context(user_api_key_dict, call_type);
    during_call_hook(&harness.execution, capabilities, &mut data, scope).await
}

#[rstest]
#[case::neither(GuardrailCapabilities { has_guardrail: false, has_moderation_override: false }, &[])]
#[case::guardrail_only(GuardrailCapabilities { has_guardrail: true, has_moderation_override: false }, &["Moderator:moderation", "TestGuardrail:moderation"])]
#[case::moderation_override_only(GuardrailCapabilities { has_guardrail: false, has_moderation_override: true }, &["Moderator:moderation", "TestGuardrail:moderation"])]
#[tokio::test]
async fn no_guardrail_and_no_moderation_override_returns_at_once(
    #[case] flags: GuardrailCapabilities,
    #[case] expected: &[&str],
) {
    let harness = harness();
    let mut moderator = fake("Moderator", None, Answer::Pass, &harness.log);
    moderator.moderation_override = true;
    let mut listed = capabilities(vec![
        shaped(Class::Plain, Answer::Pass, &harness),
        moderator,
    ]);
    listed.guardrails = flags;
    let user = auth();
    let outcome = run(&harness, &listed, Some(&user), CallType::Acompletion).await;
    assert_eq!(outcome, Ok(()));
    let mut calls = harness.log.entries();
    calls.sort();
    assert_eq!(calls, expected);
}

#[rstest]
#[case::v1_pre_call_check(false, Some("pre_call"), Ok(true), &[], &[])]
#[case::v1_during_call_check(false, Some("during_call"), Ok(false), &["TestGuardrail:moderation"], &[])]
#[case::v1_without_check(false, None, Ok(true), &["TestGuardrail:moderation"], &[GuardrailEvent::DuringCall])]
#[case::event_hook_gate_false(true, Some("pre_call"), Ok(false), &[], &[GuardrailEvent::DuringCall])]
#[case::event_hook_gate_true(true, Some("pre_call"), Ok(true), &["TestGuardrail:moderation"], &[GuardrailEvent::DuringCall])]
#[tokio::test]
async fn the_during_call_selection_follows_litellm(
    #[case] event_hook_set: bool,
    #[case] moderation_check: Option<&'static str>,
    #[case] gate: Result<bool, GuardrailGateError>,
    #[case] expected_calls: &[&str],
    #[case] expected_events: &[GuardrailEvent],
) {
    let harness = harness();
    let mut held = guardrail(Some(NAME));
    held.event_hook_set = event_hook_set;
    held.moderation_check = moderation_check;
    held.gate = gate;
    let callback = Arc::new(fake(CLASS, Some(held), Answer::Pass, &harness.log));
    let listed: Arc<dyn ProxyCallback> = Arc::<Fake>::clone(&callback);
    let selected = CallbackCapabilities::from_callbacks(&[listed]);
    let user = auth();
    let outcome = run(&harness, &selected, Some(&user), CallType::Acompletion).await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(harness.log.entries(), expected_calls);
    let asked = callback.guardrail.as_ref().map(|gated| {
        gated
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    });
    assert_eq!(asked, Some(expected_events.to_vec()));
}

#[tokio::test]
async fn a_during_call_gate_error_fails_the_call() {
    let harness = harness();
    let mut held = guardrail(Some(NAME));
    held.gate = Err(GuardrailGateError::RequestedNotList);
    let listed = capabilities(vec![fake(CLASS, Some(held), Answer::Pass, &harness.log)]);
    let user = auth();
    let outcome = run(&harness, &listed, Some(&user), CallType::Acompletion).await;
    let expected = GuardrailGateError::RequestedNotList.to_string();
    assert_eq!(outcome, Err(ProxyHookError::Other(expected)));
    assert_eq!(harness.log.entries(), Vec::<String>::new());
}

#[rstest]
#[case::call_mcp_tool(CallType::CallMcpTool, GuardrailEvent::DuringMcpCall)]
#[case::list_mcp_tools(CallType::ListMcpTools, GuardrailEvent::DuringCall)]
#[case::chat(CallType::Acompletion, GuardrailEvent::DuringCall)]
#[tokio::test]
async fn the_during_call_gate_hears_the_mcp_event_only_for_tool_calls(
    #[case] call_type: CallType,
    #[case] expected: GuardrailEvent,
) {
    let harness = harness();
    let callback = Arc::new(shaped(Class::Plain, Answer::Pass, &harness));
    let listed: Arc<dyn ProxyCallback> = Arc::<Fake>::clone(&callback);
    let selected = CallbackCapabilities::from_callbacks(&[listed]);
    let user = auth();
    let outcome = run(&harness, &selected, Some(&user), call_type).await;
    assert_eq!(outcome, Ok(()));
    let asked = callback.guardrail.as_ref().map(|gated| {
        gated
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    });
    assert_eq!(asked, Some(vec![expected]));
}

#[rstest]
#[case::with_key(true, Some(auth()), &["Moderator:moderation"])]
#[case::without_key(true, None, &[])]
#[case::no_override(false, Some(auth()), &[])]
#[tokio::test]
async fn a_moderation_override_runs_only_with_the_callers_key(
    #[case] moderation_override: bool,
    #[case] user: Option<UserAPIKeyAuth>,
    #[case] expected: &[&str],
) {
    let harness = harness();
    let mut moderator = fake("Moderator", None, Answer::Pass, &harness.log);
    moderator.moderation_override = moderation_override;
    let mut listed = capabilities(vec![moderator]);
    listed.guardrails.has_moderation_override = true;
    let outcome = run(&harness, &listed, user.as_ref(), CallType::Acompletion).await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(harness.log.entries(), expected);
}

#[rstest]
#[case::unified_with_key(Class::Unified, Some(auth()), "unified:moderation:TestGuardrail")]
#[case::unified_without_key(Class::Unified, None, "TestGuardrail:moderation")]
#[case::native_during_call_hook(Class::NativeDuringCall, Some(auth()), "TestGuardrail:moderation")]
#[case::plain_without_key(Class::Plain, None, "TestGuardrail:moderation")]
#[tokio::test]
async fn the_unified_moderation_needs_a_key_and_no_native_during_call_hook(
    #[case] class: Class,
    #[case] user: Option<UserAPIKeyAuth>,
    #[case] expected: &str,
) {
    let harness = harness();
    let listed = capabilities(vec![shaped(class, Answer::Pass, &harness)]);
    let outcome = run(&harness, &listed, user.as_ref(), CallType::Acompletion).await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(harness.log.entries(), vec![expected.to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn the_first_during_call_error_is_raised_and_the_rest_are_dropped() {
    let harness = harness();
    let mut slow = fake(
        "Slow",
        Some(guardrail(Some("slow"))),
        Answer::Fail(http("slow")),
        &harness.log,
    );
    slow.delay = Duration::from_millis(50);
    let fast = fake(
        "Fast",
        Some(guardrail(Some("fast"))),
        Answer::Fail(http("fast")),
        &harness.log,
    );
    let listed = capabilities(vec![slow, fast]);
    let user = auth();
    let outcome = run(&harness, &listed, Some(&user), CallType::Acompletion).await;
    assert_eq!(outcome, Err(http("fast")));
    assert_eq!(harness.log.entries(), vec!["Fast:moderation".to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn every_selected_during_call_hook_runs_and_a_raise_is_recorded() {
    let harness = harness();
    let passing = fake(
        "Passing",
        Some(guardrail(Some("passing"))),
        Answer::Pass,
        &harness.log,
    );
    let mut blocking = shaped(Class::Plain, Answer::Fail(http("blocked")), &harness);
    blocking.delay = Duration::from_millis(10);
    let listed = capabilities(vec![passing, blocking]);
    let user = auth();
    let mut data = map("{}");
    let scope = context(Some(&user), CallType::Acompletion);
    let outcome = during_call_hook(&harness.execution, &listed, &mut data, scope).await;
    assert_eq!(outcome, Err(http("blocked")));
    let mut calls = harness.log.entries();
    calls.sort();
    assert_eq!(
        calls,
        vec![
            "Passing:moderation".to_owned(),
            "TestGuardrail:moderation".to_owned()
        ]
    );
    assert_eq!(
        metadata_field(&data, "applied_guardrails"),
        Some(&strings(&[NAME]))
    );
}

#[rstest]
#[case::named(Some(NAME), Answer::Pass, NAME, GuardrailStatus::Success, None)]
#[case::unnamed(None, Answer::Pass, CLASS, GuardrailStatus::Success, None)]
#[case::failed(
    Some(NAME),
    Answer::Fail(http("blocked")),
    NAME,
    GuardrailStatus::Error,
    Some("HTTPException")
)]
#[tokio::test]
async fn a_during_call_guardrail_reports_under_its_name_or_class(
    #[case] guardrail_name: Option<&'static str>,
    #[case] answer: Answer,
    #[case] expected_name: &str,
    #[case] status: GuardrailStatus,
    #[case] error_type: Option<&str>,
) {
    let harness = harness();
    let listed = capabilities(vec![fake(
        CLASS,
        Some(guardrail(guardrail_name)),
        answer,
        &harness.log,
    )]);
    let user = auth();
    let outcome = run(&harness, &listed, Some(&user), CallType::Acompletion).await;
    assert_eq!(outcome.is_ok(), status == GuardrailStatus::Success);
    let expected = Observed {
        name: expected_name.to_owned(),
        latency: Decimal::ONE,
        status,
        error_type: error_type.map(str::to_owned),
        hook_type: "during_call".to_owned(),
    };
    assert_eq!(harness.metrics.observed(), vec![expected]);
}
