#![cfg(test)]

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rstest::rstest;

use super::super::tests::support::{
    CLEAN, NAME, Recorder, armor, armor_with, data, prompt_data, settings, settings_for, value,
};
use super::HookPhase;

#[rstest]
#[case::plain(None, GuardrailEventHooks::PreCall)]
#[case::completion(Some("completion"), GuardrailEventHooks::PreCall)]
#[case::mcp(Some("call_mcp_tool"), GuardrailEventHooks::PreMcpCall)]
fn events_pre_call_maps_the_call_type(
    #[case] call_type: Option<&str>,
    #[case] expected: GuardrailEventHooks,
) {
    assert_eq!(HookPhase::PreCall.event_type(call_type), expected);
}

#[rstest]
#[case::plain(None, GuardrailEventHooks::DuringCall)]
#[case::completion(Some("completion"), GuardrailEventHooks::DuringCall)]
#[case::mcp(Some("call_mcp_tool"), GuardrailEventHooks::DuringMcpCall)]
fn events_moderation_maps_the_call_type(
    #[case] call_type: Option<&str>,
    #[case] expected: GuardrailEventHooks,
) {
    assert_eq!(HookPhase::Moderation.event_type(call_type), expected);
}

#[test]
fn events_hook_names_match_the_litellm_error_labels() {
    assert_eq!(HookPhase::PreCall.hook_name(), "pre-call");
    assert_eq!(HookPhase::Moderation.hook_name(), "moderation");
}

#[tokio::test]
async fn events_a_configured_hook_runs_only_for_its_event() {
    let fixture = armor_with(
        vec![(StatusCode::OK, CLEAN.to_owned())],
        settings_for(&["during_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
    let mut during = Recorder::new(Some(GuardrailEventHooks::DuringCall));
    hooks
        .moderation(&mut request, None, &mut during.context())
        .await
        .expect("runs");
    assert_eq!(fixture.calls(), 1);
}

#[tokio::test]
async fn events_the_mcp_hook_runs_only_for_mcp_calls() {
    let fixture = armor_with(
        vec![(StatusCode::OK, CLEAN.to_owned())],
        settings_for(&["pre_mcp_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreMcpCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
    let mut mcp = Recorder::new(Some(GuardrailEventHooks::PreMcpCall));
    hooks
        .pre_call(&mut request, Some("call_mcp_tool"), &mut mcp.context())
        .await
        .expect("runs");
    assert_eq!(fixture.calls(), 1);
}

#[tokio::test]
async fn events_a_suppressed_guardrail_is_skipped() {
    let fixture = armor(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    recorder.suppressed.insert(NAME.to_owned());
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
}

#[rstest]
#[case::opted_out(r#"{"user_api_key_metadata":{"opted_out_global_guardrails":["armor"]}}"#)]
#[case::disabled(r#"{"user_api_key_metadata":{"disable_global_guardrails":true}}"#)]
#[case::team(r#"{"user_api_key_team_metadata":{"disable_global_guardrails":true}}"#)]
#[tokio::test]
async fn events_an_opted_out_global_guardrail_is_skipped(#[case] admin: &str) {
    let fixture = armor(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    request.insert("metadata".to_owned(), value(admin));
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
}

#[tokio::test]
async fn events_a_guardrail_that_is_not_default_on_needs_a_requested_entry() {
    let mut base = settings();
    base.default_on = false;
    let fixture = armor_with(vec![(StatusCode::OK, CLEAN.to_owned())], base, false).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
    request.insert("guardrails".to_owned(), value(r#"["armor"]"#));
    let mut requested = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut requested.context())
        .await
        .expect("runs");
    assert_eq!(fixture.calls(), 1);
}

#[tokio::test]
async fn events_a_requested_guardrail_is_matched_by_name_only() {
    let mut base = settings();
    base.default_on = false;
    let fixture = armor_with(vec![(StatusCode::OK, CLEAN.to_owned())], base, false).await;
    let hooks = fixture.hooks();
    let mut request =
        data(r#"{"messages":[{"role":"user","content":"hello"}],"guardrails":["other"]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
}
