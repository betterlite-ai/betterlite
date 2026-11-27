#![cfg(test)]

use betterlite_oracle_helper_api::helper::Helper as _;
use serde_json::{Value, json};

use super::Op;
use super::input::Input;

fn team_labels() -> Value {
    json!({
        "litellm_remaining_team_budget_metric": ["team", "team_alias"],
        "litellm_team_max_budget_metric": ["team", "team_alias"],
        "litellm_team_budget_remaining_hours_metric": ["team", "team_alias"]
    })
}

fn run(input: Value) -> Value {
    let parsed = serde_json::from_value::<Input>(input).unwrap();
    Op::run(&parsed).unwrap()
}

#[test]
fn a_carried_team_snapshot_skips_the_lookup() {
    let output = run(json!({
        "function": "team",
        "team_id": "team-1",
        "team_alias": "Alpha",
        "team_spend": 1.5,
        "response_cost": 0.25,
        "carried": {"budget_reset_at": "2026-01-06T00:00:00Z", "max_budget": 20},
        "lookup": {"kind": "missing"},
        "now": "2026-01-01T00:00:00Z",
        "noop": false,
        "labels_by_metric": team_labels()
    }));
    assert_eq!(
        output,
        json!({
            "samples": [
                {
                    "metric": "litellm_remaining_team_budget_metric",
                    "labels": {"team": "team-1", "team_alias": "Alpha"},
                    "value": "18.250000"
                },
                {
                    "metric": "litellm_team_max_budget_metric",
                    "labels": {"team": "team-1", "team_alias": "Alpha"},
                    "value": "20.000000"
                },
                {
                    "metric": "litellm_team_budget_remaining_hours_metric",
                    "labels": {"team": "team-1", "team_alias": "Alpha"},
                    "value": "120.000000"
                }
            ],
            "lookups": []
        })
    );
}

#[test]
fn a_fetched_team_record_fills_the_missing_max_budget() {
    let output = run(json!({
        "function": "team",
        "team_id": "team-1",
        "team_alias": "Alpha",
        "team_spend": 2,
        "response_cost": 0.25,
        "lookup": {
            "kind": "record",
            "budget_reset_at": "2026-01-06T00:00:00Z",
            "max_budget": 30
        },
        "now": "2026-01-01T00:00:00Z",
        "noop": false,
        "labels_by_metric": team_labels()
    }));
    assert_eq!(
        output,
        json!({
            "samples": [
                {
                    "metric": "litellm_remaining_team_budget_metric",
                    "labels": {"team": "team-1", "team_alias": "Alpha"},
                    "value": "27.750000"
                },
                {
                    "metric": "litellm_team_max_budget_metric",
                    "labels": {"team": "team-1", "team_alias": "Alpha"},
                    "value": "30.000000"
                },
                {
                    "metric": "litellm_team_budget_remaining_hours_metric",
                    "labels": {"team": "team-1", "team_alias": "Alpha"},
                    "value": "120.000000"
                }
            ],
            "lookups": [{"name": "team_info", "args": {"team_id": "team-1"}}]
        })
    );
}

#[test]
fn the_key_fallback_reads_the_cache_only() {
    let output = run(json!({
        "function": "key",
        "user_api_key": "key-1",
        "user_api_key_alias": "Key Alpha",
        "response_cost": 0.25,
        "key_spend": 1,
        "lookup": {"kind": "record", "budget_reset_at": "2026-01-06T00:00:00Z"},
        "now": "2026-01-01T00:00:00Z",
        "noop": false,
        "labels_by_metric": {
            "litellm_remaining_api_key_budget_metric": ["hashed_api_key", "api_key_alias"],
            "litellm_api_key_max_budget_metric": ["hashed_api_key", "api_key_alias"],
            "litellm_api_key_budget_remaining_hours_metric": ["hashed_api_key", "api_key_alias"]
        }
    }));
    assert_eq!(
        output,
        json!({
            "samples": [
                {
                    "metric": "litellm_remaining_api_key_budget_metric",
                    "labels": {"hashed_api_key": "key-1", "api_key_alias": "Key Alpha"},
                    "value": "inf"
                },
                {
                    "metric": "litellm_api_key_budget_remaining_hours_metric",
                    "labels": {"hashed_api_key": "key-1", "api_key_alias": "Key Alpha"},
                    "value": "120.000000"
                }
            ],
            "lookups": [{
                "name": "key_info",
                "args": {"hashed_token": "key-1", "check_cache_only": true}
            }]
        })
    );
}

#[test]
fn a_customer_cache_hit_reports_no_reset_hours() {
    let output = run(json!({
        "function": "customer",
        "end_user_id": "end-1",
        "response_cost": 0.25,
        "enable_prometheus_only": true,
        "lookup": {
            "kind": "record",
            "spend": 1.5,
            "budget": {"max_budget": 5, "budget_reset_at": "2026-01-06T00:00:00Z"}
        },
        "now": "2026-01-01T00:00:00Z",
        "noop": false,
        "labels_by_metric": {
            "litellm_remaining_customer_budget_metric": ["end_user"],
            "litellm_customer_max_budget_metric": ["end_user"],
            "litellm_customer_budget_remaining_hours_metric": ["end_user"]
        }
    }));
    assert_eq!(
        output,
        json!({
            "samples": [
                {
                    "metric": "litellm_remaining_customer_budget_metric",
                    "labels": {"end_user": "end-1"},
                    "value": "3.250000"
                },
                {
                    "metric": "litellm_customer_max_budget_metric",
                    "labels": {"end_user": "end-1"},
                    "value": "5.000000"
                }
            ],
            "lookups": [{
                "name": "cached_customer",
                "args": {"cache_key": "end_user_id:end-1"}
            }]
        })
    );
}

#[test]
fn the_customer_gate_is_off_by_default() {
    let output = run(json!({
        "function": "customer",
        "end_user_id": "end-3",
        "response_cost": 0.25,
        "lookup": {"kind": "record", "spend": 1, "budget": {"max_budget": 5}},
        "now": "2026-01-01T00:00:00Z",
        "noop": false,
        "labels_by_metric": {
            "litellm_remaining_customer_budget_metric": ["end_user"],
            "litellm_customer_max_budget_metric": ["end_user"],
            "litellm_customer_budget_remaining_hours_metric": ["end_user"]
        }
    }));
    assert_eq!(output, json!({"samples": [], "lookups": []}));
}

#[test]
fn no_default_budget_id_reads_nothing() {
    let output = run(json!({
        "function": "default_customer_budget",
        "default_budget_id": null,
        "lookup": {"kind": "missing"}
    }));
    assert_eq!(output, json!({"budget": null, "lookups": []}));
}

#[test]
fn a_default_customer_budget_reports_its_limit_and_reset() {
    let output = run(json!({
        "function": "default_customer_budget",
        "default_budget_id": "budget-1",
        "lookup": {
            "kind": "record",
            "budget": {"max_budget": 25, "budget_reset_at": "2026-02-01T00:00:00Z"}
        }
    }));
    assert_eq!(
        output,
        json!({
            "budget": {"max_budget": "25.000000", "budget_reset_at": "1769904000"},
            "lookups": [{
                "name": "default_customer_budget",
                "args": {"budget_id": "budget-1"}
            }]
        })
    );
}

#[test]
fn noop_gauges_skip_before_the_lookup() {
    let output = run(json!({
        "function": "team",
        "team_id": "team-1",
        "team_alias": "Alpha",
        "team_spend": 1,
        "response_cost": 0.25,
        "lookup": {"kind": "record", "max_budget": 10},
        "now": "2026-01-01T00:00:00Z",
        "noop": true,
        "labels_by_metric": team_labels()
    }));
    assert_eq!(output, json!({"samples": [], "lookups": []}));
}
