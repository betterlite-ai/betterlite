#![cfg(test)]

use serde_json::{Value, json};

use crate::helpers::Helper as _;

use super::Op;

fn run(input: &Value) -> Value {
    Op::decode_and_run(input).expect("the helper runs")
}

fn principal(key_limit: i64, team_limit: i64) -> Value {
    json!({
        "api_key": "sk-oracle-key",
        "team_id": "team-oracle",
        "metadata": {"batch_enqueued_token_limit": key_limit},
        "team_metadata": {"batch_enqueued_token_limit": team_limit},
    })
}

fn reserve(key: &str, limit: i64) -> Value {
    json!({
        "event": "reserve",
        "key": key,
        "args": [500_i64, 691_200_i64, limit],
    })
}

const KEY_COUNTER: &str = "batch_enqueued_tokens:api_key:sk-oracle-key";
const TEAM_COUNTER: &str = "batch_enqueued_tokens:team:team-oracle";

#[test]
fn key_team_reserve_matches_the_pinned_answer() {
    let answer = run(&json!({
        "tokens": 500_i64,
        "principal": principal(1000, 2000),
        "redis": {"reserve": [[1_i64, 500_i64], [1_i64, 500_i64]], "refund": []},
        "memory": {"seed": {}, "read": []},
    }));
    assert_eq!(
        answer["outcome"],
        json!({
            "kind": "reservation",
            "tokens": 500_i64,
            "scopes": [
                {"key": "api_key", "value": "sk-oracle-key", "limit": 1000_i64},
                {"key": "team", "value": "team-oracle", "limit": 2000_i64},
            ],
            "backend": "redis",
            "owner_present": false,
        })
    );
    assert_eq!(
        answer["events"],
        json!([reserve(KEY_COUNTER, 1000), reserve(TEAM_COUNTER, 2000)])
    );
    assert_eq!(answer["memory"], json!({}));
}

#[test]
fn enqueued_limit_exceeded_matches_the_pinned_answer() {
    let answer = run(&json!({
        "tokens": 500_i64,
        "principal": principal(1000, 2000),
        "redis": {"reserve": [[0_i64, 950_i64]], "refund": []},
        "memory": {"seed": {}, "read": []},
    }));
    assert_eq!(
        answer["outcome"],
        json!({
            "kind": "over_limit",
            "scope": {"key": "api_key", "value": "sk-oracle-key", "limit": 1000_i64},
            "enqueued": 950_i64,
        })
    );
    assert_eq!(answer["events"], json!([reserve(KEY_COUNTER, 1000)]));
    assert_eq!(answer["memory"], json!({}));
}

#[test]
fn invalid_limit_warning_matches_the_pinned_answer() {
    let answer = run(&json!({
        "tokens": 500_i64,
        "principal": principal(0, -3),
        "redis": null,
        "memory": {"seed": {}, "read": []},
    }));
    assert_eq!(
        answer["outcome"],
        json!({
            "kind": "reservation",
            "tokens": 500_i64,
            "scopes": [],
            "backend": "redis",
            "owner_present": false,
        })
    );
    assert_eq!(
        answer["events"],
        json!([
            {"event": "warning", "level": "warning"},
            {"event": "warning", "level": "warning"},
        ])
    );
    assert_eq!(answer["memory"], json!({}));
}

#[test]
fn partial_reserve_rollback_matches_the_pinned_answer() {
    let answer = run(&json!({
        "tokens": 500_i64,
        "principal": principal(1000, 2000),
        "redis": {"reserve": [[1_i64, 100_i64], [0_i64, 1900_i64]], "refund": [1_i64]},
        "memory": {"seed": {}, "read": []},
    }));
    assert_eq!(
        answer["outcome"],
        json!({
            "kind": "over_limit",
            "scope": {"key": "team", "value": "team-oracle", "limit": 2000_i64},
            "enqueued": 1900_i64,
        })
    );
    assert_eq!(
        answer["events"],
        json!([
            reserve(KEY_COUNTER, 1000),
            reserve(TEAM_COUNTER, 2000),
            {"event": "refund", "key": KEY_COUNTER, "args": [500_i64]},
        ])
    );
    assert_eq!(answer["memory"], json!({}));
}

#[test]
fn redis_fallback_memory_matches_the_pinned_answer() {
    let answer = run(&json!({
        "tokens": 500_i64,
        "principal": principal(1000, 2000),
        "redis": {"reserve": ["redis unavailable"], "refund": []},
        "memory": {
            "seed": {
                KEY_COUNTER: 300_i64,
                TEAM_COUNTER: 100_i64,
            },
            "read": [KEY_COUNTER, TEAM_COUNTER],
        },
    }));
    assert_eq!(
        answer["outcome"],
        json!({
            "kind": "reservation",
            "tokens": 500_i64,
            "scopes": [
                {"key": "api_key", "value": "sk-oracle-key", "limit": 1000_i64},
                {"key": "team", "value": "team-oracle", "limit": 2000_i64},
            ],
            "backend": "memory",
            "owner_present": true,
        })
    );
    assert_eq!(
        answer["events"],
        json!([reserve(KEY_COUNTER, 1000), {"event": "warning", "level": "warning"}])
    );
    assert_eq!(
        answer["memory"],
        json!({KEY_COUNTER: 800_i64, TEAM_COUNTER: 600_i64})
    );
}
