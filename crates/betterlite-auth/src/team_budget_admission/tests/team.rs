#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use chrono::DateTime;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::auth_request_models::RequestedModels;
use crate::budget_counter::CounterEntityType;

use super::super::alerts::TeamAlertKind;
use super::super::error::TeamBudgetError;
use super::super::soft::{SoftOutcome, soft_budget_recipients, team_soft_budget_check};
use super::super::sources::Membership;
use super::{Harness, Read, key, money, request, run, team};

const TEAM_KEY: &str = r#"{"token":"sk-team","user_id":"user-1","team_id":"token-team","team_alias":"Token alias","org_id":"token-org","key_alias":"team-key"}"#;

fn metadata(fields: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(fields).unwrap()
}

#[rstest]
#[case::row_spend(r#""spend":4,"#, "4")]
#[case::null_spend(r#""spend":null,"#, "0")]
#[tokio::test]
async fn hard_equality_passes(#[case] spend_field: &str, #[case] fallback: &str) {
    let harness = Harness::spending(&["10"]);
    let row = team(&format!(
        r#"{{"team_id":"team-1",{spend_field}"max_budget":10}}"#
    ));
    let token = key(TEAM_KEY);
    let model = RequestedModels::None;
    run(
        &harness,
        &request(Some(&row), Some(&token), &model, Membership::Loaded(None)),
    )
    .await
    .unwrap();
    assert!(harness.team_alerts().is_empty());
    assert_eq!(
        harness.reads(),
        vec![Read {
            counter_key: "spend:team:team-1".to_owned(),
            fallback: money(fallback),
            max_budget: Some(money("10")),
            entity_type: None,
            entity_id: None,
            duration: None,
            start: None,
        }]
    );
}

#[tokio::test]
async fn hard_overage_alert_before_denial() {
    let harness = Harness::spending(&["10.01"]);
    let row = team(
        r#"{"team_id":"team-1","team_alias":"Row alias","organization_id":"row-org","max_budget":10,"soft_budget":1,"spend":2}"#,
    );
    let token = key(TEAM_KEY);
    let model = RequestedModels::None;
    let result = run(
        &harness,
        &request(Some(&row), Some(&token), &model, Membership::Loaded(None)),
    )
    .await;
    let Err(TeamBudgetError::Denied(denial)) = result else {
        panic!("expected the team denial, got {result:?}");
    };
    assert_eq!(denial.entity_type, "team");
    assert_eq!(denial.entity_id.unwrap().expose_secret(), "team-1");
    assert_eq!(denial.current_cost, money("10.01"));
    assert_eq!(denial.max_budget, money("10"));
    assert_eq!(
        denial.message,
        "Budget has been exceeded! Team=team-1 Current cost: 10.01, Max budget: 10"
    );
    assert_eq!(
        harness.events(),
        vec![
            "read spend:team:team-1".to_owned(),
            "team alert TeamBudget".to_owned()
        ]
    );
    let alerts = harness.team_alerts();
    let [alert] = alerts.as_slice() else {
        panic!("one team alert, got {alerts:?}");
    };
    assert_eq!(alert.spend, money("10.01"));
    assert_eq!(alert.max_budget, Some(money("10")));
    assert_eq!(alert.soft_budget, None);
    assert_eq!(alert.token.as_ref().unwrap().expose_secret(), "sk-team");
    assert_eq!(alert.user_id.as_deref(), Some("user-1"));
    assert_eq!(alert.team_id.as_deref(), Some("token-team"));
    assert_eq!(alert.team_alias.as_deref(), Some("Token alias"));
    assert_eq!(alert.organization_id.as_deref(), Some("token-org"));
    assert_eq!(alert.key_alias, None);
    assert_eq!(alert.alert_emails, None);
}

#[tokio::test]
async fn hard_without_token() {
    let harness = Harness::spending(&["11"]);
    let row = team(r#"{"team_id":"team-1","max_budget":10}"#);
    let model = RequestedModels::None;
    let result = run(
        &harness,
        &request(Some(&row), None, &model, Membership::Unloaded),
    )
    .await;
    assert!(
        matches!(result, Err(TeamBudgetError::Denied(_))),
        "{result:?}"
    );
    assert!(harness.team_alerts().is_empty());
}

#[tokio::test]
async fn hard_null_skips() {
    let harness = Harness::default();
    let row = team(r#"{"team_id":"team-1","max_budget":null,"spend":100}"#);
    let token = key(TEAM_KEY);
    let model = RequestedModels::None;
    run(
        &harness,
        &request(Some(&row), Some(&token), &model, Membership::Loaded(None)),
    )
    .await
    .unwrap();
    assert!(harness.reads().is_empty());
}

#[rstest]
#[case::empty_list(r#""budget_limits":[]"#)]
#[case::null_list(r#""budget_limits":null"#)]
#[case::absent_list(r#""team_alias":"no limits""#)]
#[tokio::test]
async fn windows_empty_skips(#[case] limits: &str) {
    let harness = Harness::default();
    let row = team(&format!(r#"{{"team_id":"team-1",{limits}}}"#));
    let model = RequestedModels::None;
    run(
        &harness,
        &request(Some(&row), None, &model, Membership::Unloaded),
    )
    .await
    .unwrap();
    assert!(harness.reads().is_empty());
}

#[rstest]
#[case::day_first(r#"[{"budget_duration":"1d","max_budget":50},{"budget_duration":"1h","max_budget":5}]"#, ["1d", "1h"])]
#[case::hour_first(r#"[{"budget_duration":"1h","max_budget":5},{"budget_duration":"1d","max_budget":50}]"#, ["1h", "1d"])]
#[tokio::test]
async fn window_duration_keys_survive_reordering(#[case] limits: &str, #[case] order: [&str; 2]) {
    let harness = Harness::spending(&["0", "0"]);
    let row = team(&format!(
        r#"{{"team_id":"team-1","budget_limits":{limits}}}"#
    ));
    let model = RequestedModels::None;
    run(
        &harness,
        &request(Some(&row), None, &model, Membership::Unloaded),
    )
    .await
    .unwrap();
    let keys: Vec<String> = harness
        .reads()
        .into_iter()
        .map(|read| read.counter_key)
        .collect();
    let expected: Vec<String> = order
        .iter()
        .map(|duration| format!("spend:team:team-1:window:{duration}"))
        .collect();
    assert_eq!(keys, expected);
}

#[rstest]
#[case::aware_reset(
    r#","reset_at":"2026-06-02T00:00:00+02:00""#,
    Some("2026-06-01T00:00:00+02:00")
)]
#[case::no_reset("", Some("2026-05-31T12:00:00+00:00"))]
#[case::unknown_duration_unit("", None)]
#[tokio::test]
async fn window_read_metadata(#[case] reset: &str, #[case] start: Option<&str>) {
    let duration = if start.is_none() { "1w" } else { "1d" };
    let harness = Harness::spending(&["0"]);
    let row = team(&format!(
        r#"{{"team_id":"team-1","budget_limits":[{{"budget_duration":"{duration}","max_budget":50{reset}}}]}}"#
    ));
    let model = RequestedModels::None;
    run(
        &harness,
        &request(Some(&row), None, &model, Membership::Unloaded),
    )
    .await
    .unwrap();
    assert_eq!(
        harness.reads(),
        vec![Read {
            counter_key: format!("spend:team:team-1:window:{duration}"),
            fallback: money("0"),
            max_budget: Some(money("50")),
            entity_type: Some(CounterEntityType::Team),
            entity_id: Some("team-1".to_owned()),
            duration: Some(duration.to_owned()),
            start: start.map(|text| Timestamp::Aware(DateTime::parse_from_rfc3339(text).unwrap())),
        }]
    );
}

#[rstest]
#[case::below("4.99", None)]
#[case::equal(
    "5",
    Some("ExceededBudget: Team=team-1 over 1h budget. Spend=$5.0000, Limit=$5.00")
)]
#[tokio::test]
async fn first_window_equality_stops_reads(#[case] spend: &str, #[case] message: Option<&str>) {
    let harness = Harness::spending(&[spend, "0"]);
    let row = team(
        r#"{"team_id":"team-1","budget_limits":[{"budget_duration":"1h","max_budget":5},{"budget_duration":"1d","max_budget":50}]}"#,
    );
    let token = key(TEAM_KEY);
    let model = RequestedModels::None;
    let result = run(
        &harness,
        &request(Some(&row), Some(&token), &model, Membership::Loaded(None)),
    )
    .await;
    match (message, result) {
        (None, Ok(_)) => assert_eq!(harness.reads().len(), 2),
        (Some(expected), Err(TeamBudgetError::Denied(denial))) => {
            assert_eq!(denial.message, expected);
            assert_eq!(denial.entity_type, "team");
            assert_eq!(denial.entity_id.unwrap().expose_secret(), "team-1");
            assert_eq!(harness.reads().len(), 1);
        }
        (_, other) => panic!("unexpected window result {other:?}"),
    }
    assert!(harness.team_alerts().is_empty());
}

#[tokio::test]
async fn soft_row_spend_only() {
    // no reader spend is scripted, so a counter read would panic
    let harness = Harness::default();
    let row = team(
        r#"{"team_id":"team-1","soft_budget":5,"spend":6,"metadata":{"soft_budget_alerting_emails":"ops@example.com"}}"#,
    );
    let token = key(TEAM_KEY);
    let model = RequestedModels::None;
    let outcome = run(
        &harness,
        &request(Some(&row), Some(&token), &model, Membership::Loaded(None)),
    )
    .await
    .unwrap();
    assert_eq!(outcome, SoftOutcome::Alerted);
    assert!(harness.reads().is_empty());
    assert_eq!(harness.team_alerts()[0].spend, money("6"));
}

#[rstest]
#[case::below("4.99", SoftOutcome::NotCrossed)]
#[case::equal("5", SoftOutcome::Alerted)]
#[case::above("9", SoftOutcome::Alerted)]
#[tokio::test]
async fn soft_equality_continues(#[case] spend: &str, #[case] outcome: SoftOutcome) {
    let harness = Harness::default();
    let row = team(&format!(
        r#"{{"team_id":"team-1","soft_budget":5,"spend":{spend},"metadata":{{"soft_budget_alerting_emails":["ops@example.com"]}}}}"#
    ));
    let token = key(TEAM_KEY);
    let model = RequestedModels::None;
    let result = run(
        &harness,
        &request(Some(&row), Some(&token), &model, Membership::Loaded(None)),
    )
    .await;
    assert_eq!(result.unwrap(), outcome);
    // a hard denial still lets the soft alert go out, as the gathered checks all settle
    let denied = Harness::spending(&["20"]);
    let over = team(&format!(
        r#"{{"team_id":"team-1","max_budget":10,"soft_budget":5,"spend":{spend},"metadata":{{"soft_budget_alerting_emails":["ops@example.com"]}}}}"#
    ));
    let hard = run(
        &denied,
        &request(Some(&over), Some(&token), &model, Membership::Loaded(None)),
    )
    .await;
    assert!(matches!(hard, Err(TeamBudgetError::Denied(_))));
    let soft_alerts = denied
        .team_alerts()
        .iter()
        .filter(|alert| alert.kind == TeamAlertKind::SoftBudget)
        .count();
    assert_eq!(soft_alerts, usize::from(outcome == SoftOutcome::Alerted));
}

#[rstest]
#[case::soft_null(r#""soft_budget":null,"spend":9"#, SoftOutcome::NotCrossed)]
#[case::spend_null(r#""soft_budget":5,"spend":null"#, SoftOutcome::NotCrossed)]
#[case::crossed(r#""soft_budget":5,"spend":9"#, SoftOutcome::CrossedWithoutToken)]
fn soft_without_token_logs(#[case] budgets: &str, #[case] outcome: SoftOutcome) {
    let harness = Harness::default();
    let row = team(&format!(
        r#"{{"team_id":"team-1",{budgets},"metadata":{{"soft_budget_alerting_emails":["ops@example.com"]}}}}"#
    ));
    assert_eq!(
        team_soft_budget_check(Some(&row), None, Some(&harness)),
        outcome
    );
    assert!(harness.team_alerts().is_empty());
}

#[test]
fn soft_list_keeps_whitespace() {
    let configured = metadata(
        r#"{"soft_budget_alerting_emails":[" ops@example.com ","","   ",5,null,"lead@example.com"]}"#,
    );
    assert_eq!(
        soft_budget_recipients(Some(&configured)),
        Some(vec![
            " ops@example.com ".to_owned(),
            "lead@example.com".to_owned()
        ])
    );
}

#[test]
fn soft_csv_trims() {
    let configured =
        metadata(r#"{"soft_budget_alerting_emails":" ops@example.com , ,lead@example.com,"}"#);
    assert_eq!(
        soft_budget_recipients(Some(&configured)),
        Some(vec![
            "ops@example.com".to_owned(),
            "lead@example.com".to_owned()
        ])
    );
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"soft_budget_alerting_emails":null}"#)]
#[case::number(r#"{"soft_budget_alerting_emails":5}"#)]
#[case::boolean(r#"{"soft_budget_alerting_emails":true}"#)]
#[case::mapping(r#"{"soft_budget_alerting_emails":{"80":["ops@example.com"]}}"#)]
#[case::empty_list(r#"{"soft_budget_alerting_emails":[]}"#)]
#[case::blank_list(r#"{"soft_budget_alerting_emails":["  ",""]}"#)]
#[case::blank_text(r#"{"soft_budget_alerting_emails":" , "}"#)]
fn soft_invalid_recipient_shapes_skip(#[case] fields: &str) {
    let harness = Harness::default();
    let row = team(&format!(
        r#"{{"team_id":"team-1","soft_budget":5,"spend":5,"metadata":{fields}}}"#
    ));
    let token = key(TEAM_KEY);
    assert_eq!(
        team_soft_budget_check(Some(&row), Some(&token), Some(&harness)),
        SoftOutcome::SkippedWithoutRecipients
    );
    assert!(harness.team_alerts().is_empty());
}

#[test]
fn global_recipients_not_used() {
    let harness = Harness::default();
    // the member and key alert maps are other alerts' recipients, and none of them stands in
    let row = team(
        r#"{"team_id":"team-1","soft_budget":5,"spend":5,"metadata":{"team_member_max_budget_alert_emails":{"50":["lead@example.com"]},"max_budget_alert_emails":{"80":["owner@example.com"]}}}"#,
    );
    let token = key(TEAM_KEY);
    assert_eq!(
        team_soft_budget_check(Some(&row), Some(&token), Some(&harness)),
        SoftOutcome::SkippedWithoutRecipients
    );
    assert!(harness.team_alerts().is_empty());
}

#[test]
fn soft_call_info() {
    let harness = Harness::default();
    let row = team(
        r#"{"team_id":"team-1","team_alias":"Row alias","organization_id":"row-org","max_budget":100,"soft_budget":50,"spend":60,"metadata":{"soft_budget_alerting_emails":"ops@example.com,lead@example.com"}}"#,
    );
    let token = key(TEAM_KEY);
    assert_eq!(
        team_soft_budget_check(Some(&row), Some(&token), Some(&harness)),
        SoftOutcome::Alerted
    );
    let alerts = harness.team_alerts();
    let [alert] = alerts.as_slice() else {
        panic!("one soft alert, got {alerts:?}");
    };
    assert_eq!(alert.kind, TeamAlertKind::SoftBudget);
    assert_eq!(alert.spend, money("60"));
    assert_eq!(alert.max_budget, Some(money("100")));
    assert_eq!(alert.soft_budget, Some(money("50")));
    assert_eq!(alert.token.as_ref().unwrap().expose_secret(), "sk-team");
    assert_eq!(alert.user_id.as_deref(), Some("user-1"));
    assert_eq!(alert.team_id.as_deref(), Some("token-team"));
    assert_eq!(alert.team_alias.as_deref(), Some("Token alias"));
    assert_eq!(alert.organization_id.as_deref(), Some("token-org"));
    assert_eq!(alert.key_alias.as_deref(), Some("team-key"));
    assert_eq!(
        alert.alert_emails,
        Some(vec![
            "ops@example.com".to_owned(),
            "lead@example.com".to_owned()
        ])
    );
    // a missing sink leaves the outcome alone
    assert_eq!(
        team_soft_budget_check(Some(&row), Some(&token), None),
        SoftOutcome::Alerted
    );
}
