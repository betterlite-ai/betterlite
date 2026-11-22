use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;

use crate::prometheus_remaining_budget::fan_out::RemainingBudgetFanOut;

use super::support::{RealGauge, Recorder, collectors_with, full_metadata, metadata, request};

fn fan_out(recorder: &Recorder) -> RemainingBudgetFanOut<Recorder> {
    RemainingBudgetFanOut::new(
        recorder.clone(),
        Duration::from_secs(5),
        &collectors_with(&[RealGauge::Team]),
    )
}

#[tokio::test]
async fn the_key_lookup_gets_its_metadata() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&full_metadata()))
        .await;
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "user_api_key": "sk-hashed",
            "user_api_key_alias": "Key",
            "response_cost": 0.5,
            "key_max_budget": 5.0,
            "key_spend": 1.5,
            "carried": {"user_api_key_budget_reset_at": "2026-11-20T02:00:00Z"}
        }"#,
    )
    .unwrap();
    assert_eq!(recorder.calls()["key"], expected);
}

#[tokio::test]
async fn the_team_lookup_gets_its_metadata() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&full_metadata()))
        .await;
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "user_api_team": "team-1",
            "user_api_team_alias": "Team",
            "team_spend": 3.0,
            "team_max_budget": 10.0,
            "response_cost": 0.5,
            "carried": {
                "user_api_key_team_budget_reset_at": "2026-11-20T02:00:00Z",
                "user_api_key_team_table_max_budget": 10.0
            }
        }"#,
    )
    .unwrap();
    assert_eq!(recorder.calls()["team"], expected);
}

#[tokio::test]
async fn the_user_lookup_gets_its_metadata() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&full_metadata()))
        .await;
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "user_id": "user-1",
            "user_spend": 0.25,
            "user_max_budget": 2.0,
            "response_cost": 0.5,
            "carried": {
                "user_api_key_user_budget_reset_at": "2026-11-20T02:00:00Z",
                "user_api_key_user_table_max_budget": 2.0,
                "user_api_key_user_alias": "Alias"
            },
            "user_email": "u@example.com"
        }"#,
    )
    .unwrap();
    assert_eq!(recorder.calls()["user"], expected);
}

#[tokio::test]
async fn the_org_lookup_gets_its_metadata() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&full_metadata()))
        .await;
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "org_id": "org-1",
            "response_cost": 0.5,
            "carried": {
                "user_api_key_org_spend": 0.5,
                "user_api_key_org_max_budget": 7.0
            },
            "org_alias": "Org"
        }"#,
    )
    .unwrap();
    assert_eq!(recorder.calls()["org"], expected);
}

#[tokio::test]
async fn the_customer_lookup_gets_its_metadata() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&full_metadata()))
        .await;
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "end_user_id": "end-1",
            "response_cost": 0.5
        }"#,
    )
    .unwrap();
    assert_eq!(recorder.calls()["customer"], expected);
}

#[tokio::test]
async fn an_absent_metadata_map_leaves_the_key_lookup_empty() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&BTreeMap::new()))
        .await;
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "user_api_key": "sk-hashed",
            "user_api_key_alias": "Key",
            "response_cost": 0.5,
            "key_max_budget": null,
            "key_spend": null,
            "carried": null
        }"#,
    )
    .unwrap();
    assert_eq!(recorder.calls()["key"], expected);
}

#[tokio::test]
async fn an_absent_metadata_map_leaves_the_team_lookup_empty() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&BTreeMap::new()))
        .await;
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "user_api_team": "team-1",
            "user_api_team_alias": "Team",
            "team_spend": null,
            "team_max_budget": null,
            "response_cost": 0.5,
            "carried": null
        }"#,
    )
    .unwrap();
    assert_eq!(recorder.calls()["team"], expected);
}

#[tokio::test]
async fn an_absent_metadata_map_leaves_the_user_lookup_empty() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&BTreeMap::new()))
        .await;
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "user_id": "user-1",
            "user_spend": null,
            "user_max_budget": null,
            "response_cost": 0.5,
            "carried": null,
            "user_email": null
        }"#,
    )
    .unwrap();
    assert_eq!(recorder.calls()["user"], expected);
}

#[tokio::test]
async fn an_absent_metadata_map_leaves_the_org_lookup_empty() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&BTreeMap::new()))
        .await;
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "org_id": "org-1",
            "response_cost": 0.5,
            "carried": null,
            "org_alias": null
        }"#,
    )
    .unwrap();
    assert_eq!(recorder.calls()["org"], expected);
}

#[tokio::test]
async fn the_customer_lookup_ignores_the_metadata_map() {
    let recorder = Recorder::recording();
    fan_out(&recorder)
        .increment_remaining_budget_metrics(request(&full_metadata()))
        .await;
    let with_metadata = recorder.calls();
    let without_metadata = Recorder::recording();
    fan_out(&without_metadata)
        .increment_remaining_budget_metrics(request(&BTreeMap::new()))
        .await;
    assert_eq!(
        without_metadata.calls()["customer"],
        with_metadata["customer"]
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
    assert_eq!(calls["key"]["key_spend"], AgentJson::Null);
    assert_eq!(calls["team"]["team_max_budget"], AgentJson::Null);
    assert_eq!(calls["user"]["user_email"], AgentJson::Null);
    assert_eq!(calls["org"]["org_alias"], AgentJson::Null);
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
    assert_eq!(calls["key"]["key_spend"], AgentJson::Null);
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
    let expected: AgentJson = serde_json::from_str(
        r#"{
            "user_api_key": "sk-hashed",
            "user_api_key_alias": "Key",
            "response_cost": 0.5,
            "key_max_budget": null,
            "key_spend": null,
            "carried": {"user_api_key_budget_reset_at": "2026-11-20T02:00:00Z"}
        }"#,
    )
    .unwrap();
    assert_eq!(calls["key"], expected);
    assert_eq!(calls["team"]["carried"], AgentJson::Null);
}
