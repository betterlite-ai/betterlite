use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::proxy::carried_budget_state::{
    KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot,
};

use crate::prometheus_remaining_budget::fan_out::RemainingBudgetFanOut;

use super::support::{
    RealGauge, RecordedCall, Recorder, collectors_with, decimal, metadata, request, reset_at,
};

fn fan_out(recorder: &Recorder) -> RemainingBudgetFanOut<Recorder> {
    RemainingBudgetFanOut::new(
        recorder.clone(),
        Duration::from_secs(5),
        &collectors_with(&[RealGauge::Team]),
    )
}

#[tokio::test]
async fn each_lookup_gets_its_metadata() {
    let entries = metadata(&[
        ("user_api_key_team_spend", AgentJson::from(3.0_f64)),
        ("user_api_key_team_max_budget", AgentJson::from(10.0_f64)),
        ("user_api_key_spend", AgentJson::from(1.5_f64)),
        ("user_api_key_max_budget", AgentJson::from(5.0_f64)),
        ("user_api_key_user_spend", AgentJson::from(0.25_f64)),
        ("user_api_key_user_max_budget", AgentJson::from(2.0_f64)),
        ("user_api_key_user_email", AgentJson::from("u@example.com")),
        ("user_api_key_org_alias", AgentJson::from("Org")),
        (
            "user_api_key_budget_reset_at",
            AgentJson::from("2026-11-20T02:00:00Z"),
        ),
        (
            "user_api_key_team_budget_reset_at",
            AgentJson::from("2026-11-20T02:00:00Z"),
        ),
        (
            "user_api_key_team_table_max_budget",
            AgentJson::from(10.0_f64),
        ),
        (
            "user_api_key_user_budget_reset_at",
            AgentJson::from("2026-11-20T02:00:00Z"),
        ),
        (
            "user_api_key_user_table_max_budget",
            AgentJson::from(2.0_f64),
        ),
        ("user_api_key_user_alias", AgentJson::from("Alias")),
        ("user_api_key_org_spend", AgentJson::from(0.5_f64)),
        ("user_api_key_org_max_budget", AgentJson::from(7.0_f64)),
    ]);
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&entries))
        .await;
    assert_eq!(
        recorder.calls(),
        vec![
            RecordedCall::Key {
                key: Some("sk-hashed".to_owned()),
                key_alias: Some("Key".to_owned()),
                response_cost: decimal("0.5"),
                max_budget: Some(decimal("5.0")),
                spend: Some(decimal("1.5")),
                carried: Some(KeyBudgetSnapshot {
                    budget_reset_at: Some(reset_at()),
                }),
            },
            RecordedCall::Team {
                team: Some("team-1".to_owned()),
                team_alias: Some("Team".to_owned()),
                spend: Some(decimal("3.0")),
                max_budget: Some(decimal("10.0")),
                response_cost: decimal("0.5"),
                carried: Some(TeamBudgetSnapshot {
                    budget_reset_at: Some(reset_at()),
                    max_budget: Some(decimal("10.0")),
                }),
            },
            RecordedCall::User {
                user_id: Some("user-1".to_owned()),
                spend: Some(decimal("0.25")),
                max_budget: Some(decimal("2.0")),
                response_cost: decimal("0.5"),
                carried: Some(UserBudgetSnapshot {
                    budget_reset_at: Some(reset_at()),
                    max_budget: Some(decimal("2.0")),
                    user_alias: Some("Alias".to_owned()),
                }),
                user_email: Some("u@example.com".to_owned()),
            },
            RecordedCall::Org {
                org_id: Some("org-1".to_owned()),
                response_cost: decimal("0.5"),
                carried: Some(OrgBudgetSnapshot {
                    spend: decimal("0.5"),
                    max_budget: Some(decimal("7.0")),
                }),
                org_alias: Some("Org".to_owned()),
            },
            RecordedCall::Customer {
                end_user_id: Some("end-1".to_owned()),
                response_cost: decimal("0.5"),
            },
        ]
    );
}

#[tokio::test]
async fn an_absent_metadata_map_leaves_every_optional_field_empty() {
    let entries = BTreeMap::new();
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&entries))
        .await;
    assert_eq!(
        recorder.calls(),
        vec![
            RecordedCall::Key {
                key: Some("sk-hashed".to_owned()),
                key_alias: Some("Key".to_owned()),
                response_cost: decimal("0.5"),
                max_budget: None,
                spend: None,
                carried: None,
            },
            RecordedCall::Team {
                team: Some("team-1".to_owned()),
                team_alias: Some("Team".to_owned()),
                spend: None,
                max_budget: None,
                response_cost: decimal("0.5"),
                carried: None,
            },
            RecordedCall::User {
                user_id: Some("user-1".to_owned()),
                spend: None,
                max_budget: None,
                response_cost: decimal("0.5"),
                carried: None,
                user_email: None,
            },
            RecordedCall::Org {
                org_id: Some("org-1".to_owned()),
                response_cost: decimal("0.5"),
                carried: None,
                org_alias: None,
            },
            RecordedCall::Customer {
                end_user_id: Some("end-1".to_owned()),
                response_cost: decimal("0.5"),
            },
        ]
    );
}

#[tokio::test]
async fn drops_values_that_are_not_the_declared_type() {
    let entries = metadata(&[
        ("user_api_key_spend", AgentJson::Null),
        ("user_api_key_team_max_budget", AgentJson::from("10.0")),
        ("user_api_key_user_email", AgentJson::from(42_i64)),
        ("user_api_key_org_alias", AgentJson::from(false)),
    ]);
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&entries))
        .await;
    let calls = recorder.calls();
    assert_eq!(calls.len(), 5);
    let RecordedCall::Key { spend, .. } = &calls[0] else {
        panic!("the key lookup runs first");
    };
    assert!(spend.is_none());
    let RecordedCall::Team { max_budget, .. } = &calls[1] else {
        panic!("the team lookup runs second");
    };
    assert!(max_budget.is_none());
    let RecordedCall::User { user_email, .. } = &calls[2] else {
        panic!("the user lookup runs third");
    };
    assert!(user_email.is_none());
    let RecordedCall::Org { org_alias, .. } = &calls[3] else {
        panic!("the org lookup runs fourth");
    };
    assert!(org_alias.is_none());
}

#[tokio::test]
async fn drops_an_amount_that_does_not_fit_a_decimal() {
    let huge: AgentJson = serde_json::from_str("1e300").unwrap();
    let entries = metadata(&[("user_api_key_spend", huge)]);
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&entries))
        .await;
    let calls = recorder.calls();
    assert_eq!(calls.len(), 5);
    let RecordedCall::Key { spend, .. } = &calls[0] else {
        panic!("the key lookup runs first");
    };
    assert!(spend.is_none());
}

#[tokio::test]
async fn snapshots_parse_only_the_keys_they_carry() {
    let entries = metadata(&[
        (
            "user_api_key_budget_reset_at",
            AgentJson::from("2026-11-20T02:00:00Z"),
        ),
        (
            "user_api_key_team_budget_reset_at",
            AgentJson::from("not a timestamp"),
        ),
    ]);
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&entries))
        .await;
    let calls = recorder.calls();
    assert_eq!(
        calls[0],
        RecordedCall::Key {
            key: Some("sk-hashed".to_owned()),
            key_alias: Some("Key".to_owned()),
            response_cost: decimal("0.5"),
            max_budget: None,
            spend: None,
            carried: Some(KeyBudgetSnapshot {
                budget_reset_at: Some(reset_at()),
            }),
        }
    );
    let RecordedCall::Team { carried, .. } = &calls[1] else {
        panic!("the team lookup runs second");
    };
    assert!(carried.is_none());
}
