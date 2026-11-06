#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::support::{EVENT_CODE, settings_with};
use crate::finops::metering::event::UsageTokens;
use crate::finops::metering::lago::payload::{LagoPayloadError, build};
use crate::finops::metering::lago::settings::ChargeBy;
use crate::finops::metering::support::{MODEL, event};

#[test]
fn common_logic_fills_the_model_cost_usage_and_event_code() {
    let payload = build(&settings_with(&[]), &event()).unwrap();
    assert_eq!(payload.event.code, EVENT_CODE);
    assert_eq!(payload.event.properties.model.as_deref(), Some(MODEL));
    assert_eq!(
        payload.event.properties.response_cost,
        Some(Decimal::new(12, 4))
    );
    assert_eq!(payload.event.properties.prompt_tokens, Some(10));
    assert_eq!(payload.event.properties.completion_tokens, Some(20));
    assert_eq!(payload.event.properties.total_tokens, Some(30));
}

#[test]
fn common_logic_keeps_a_zero_usage_in_the_properties() {
    let mut call = event();
    call.usage = Some(UsageTokens {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
    });
    let payload = build(&settings_with(&[]), &call).unwrap();
    let body = serde_json::to_string(&payload).unwrap();
    assert!(body.contains(r#""prompt_tokens":0"#), "{body}");
}

#[test]
fn common_logic_omits_usage_when_the_response_has_none() {
    let mut call = event();
    call.usage = None;
    let payload = build(&settings_with(&[]), &call).unwrap();
    let body = serde_json::to_string(&payload).unwrap();
    assert!(!body.contains("prompt_tokens"), "{body}");
    assert!(!body.contains("completion_tokens"), "{body}");
    assert!(!body.contains("total_tokens"), "{body}");
}

#[rstest]
#[case::end_user_id("end_user_id", "end-user-1")]
#[case::team_id("team_id", "team-1")]
#[case::user_id("user_id", "key-user-1")]
fn common_logic_resolves_the_subscription_from_charge_by(
    #[case] charge_by: &'static str,
    #[case] expected: &'static str,
) {
    let payload = build(
        &settings_with(&[("LAGO_API_CHARGE_BY", charge_by)]),
        &event(),
    )
    .unwrap();
    assert_eq!(payload.event.external_subscription_id, expected);
}

#[test]
fn common_logic_uses_a_fresh_v4_transaction_id_for_every_event() {
    let settings = settings_with(&[]);
    let first = build(&settings, &event()).unwrap();
    let second = build(&settings, &event()).unwrap();
    assert_ne!(first.event.transaction_id, second.event.transaction_id);
    let parsed = Uuid::parse_str(&first.event.transaction_id).unwrap();
    assert_eq!(parsed.get_version_num(), 4);
}

#[test]
fn common_logic_rejects_a_missing_subscription() {
    let mut call = event();
    call.request_user = None;
    let error = build(&settings_with(&[]), &call).unwrap_err();
    assert!(matches!(
        error,
        LagoPayloadError::MissingSubscription {
            charge_by: ChargeBy::EndUserId,
            ..
        }
    ));
}

#[test]
fn common_logic_reports_every_candidate_id_when_the_subscription_is_missing() {
    let mut call = event();
    call.request_user = None;
    let error = build(&settings_with(&[]), &call).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("user_id=Some(\"key-user-1\")"),
        "{message}"
    );
    assert!(message.contains("team_id=Some(\"team-1\")"), "{message}");
}
