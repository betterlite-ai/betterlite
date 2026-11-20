#![cfg(test)]

use betterlite_oracle_helper_api::helper::Helper as _;
use serde_json::json;

use super::Op;
use super::input::Input;

#[test]
fn remaining_budget_reports_infinity_without_a_limit() {
    let input = json!({"function": "remaining_budget", "max_budget": null, "spend": 4_i32});
    let parsed = serde_json::from_value::<Input>(input).unwrap();
    assert_eq!(Op::run(&parsed).unwrap(), json!({"remaining": "inf"}));
}

#[test]
fn remaining_budget_subtracts_the_spend() {
    let input = json!({"function": "remaining_budget", "max_budget": 10_i32, "spend": 4_i32});
    let parsed = serde_json::from_value::<Input>(input).unwrap();
    assert_eq!(Op::run(&parsed).unwrap(), json!({"remaining": "6.000000"}));
}

#[test]
fn remaining_hours_reads_both_timestamps() {
    let input = json!({
        "function": "remaining_hours",
        "reset_at": "2026-11-20T02:00:00Z",
        "now": "2026-11-20T00:00:00Z"
    });
    let parsed = serde_json::from_value::<Input>(input).unwrap();
    assert_eq!(Op::run(&parsed).unwrap(), json!({"hours": "2.000000"}));
}

#[test]
fn the_team_setter_writes_all_three_samples() {
    let input = json!({
        "function": "team",
        "team_id": "t1",
        "team_alias": "Alpha",
        "spend": 4_i32,
        "max_budget": 10_i32,
        "budget_reset_at": "2026-11-20T02:00:00Z",
        "now": "2026-11-20T00:00:00Z",
        "labels_by_metric": {
            "litellm_remaining_team_budget_metric": ["team", "team_alias"],
            "litellm_team_max_budget_metric": ["team", "team_alias"],
            "litellm_team_budget_remaining_hours_metric": ["team", "team_alias"]
        }
    });
    let parsed = serde_json::from_value::<Input>(input).unwrap();
    assert_eq!(
        Op::run(&parsed).unwrap(),
        json!({"samples": [
            {
                "metric": "litellm_remaining_team_budget_metric",
                "labels": {"team": "t1", "team_alias": "Alpha"},
                "value": "6.000000"
            },
            {
                "metric": "litellm_team_budget_remaining_hours_metric",
                "labels": {"team": "t1", "team_alias": "Alpha"},
                "value": "2.000000"
            },
            {
                "metric": "litellm_team_max_budget_metric",
                "labels": {"team": "t1", "team_alias": "Alpha"},
                "value": "10.000000"
            }
        ]})
    );
}

#[test]
fn a_suppressed_customer_writes_no_samples() {
    let input = json!({
        "function": "customer",
        "end_user_id": "e1",
        "spend": 4_i32,
        "max_budget": 10_i32,
        "budget_reset_at": null,
        "now": "2026-11-20T00:00:00Z",
        "enable_end_user_tracking": false,
        "labels_by_metric": {
            "litellm_remaining_customer_budget_metric": ["end_user"],
            "litellm_customer_max_budget_metric": ["end_user"],
            "litellm_customer_budget_remaining_hours_metric": ["end_user"]
        }
    });
    let parsed = serde_json::from_value::<Input>(input).unwrap();
    assert_eq!(Op::run(&parsed).unwrap(), json!({"samples": []}));
}
