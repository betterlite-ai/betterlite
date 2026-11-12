#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::helpers::Helper as _;

use super::Op;

fn run_case(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/batch-enqueued-reserve")
        .join(name);
    let source = fs::read_to_string(&path).expect("the case reads");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("the case parses");
    Op::decode_and_run(&case.call.input).expect("the helper runs")
}

#[test]
fn key_team_reserve_matches_the_pinned_answer() {
    let answer = run_case("key-team-reserve.yaml");
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
        json!([
            {
                "event": "reserve",
                "key": "batch_enqueued_tokens:api_key:sk-oracle-key",
                "args": [500_i64, 691_200_i64, 1000_i64],
            },
            {
                "event": "reserve",
                "key": "batch_enqueued_tokens:team:team-oracle",
                "args": [500_i64, 691_200_i64, 2000_i64],
            },
        ])
    );
    assert_eq!(answer["memory"], json!({}));
}

#[test]
fn enqueued_limit_exceeded_matches_the_pinned_answer() {
    let answer = run_case("enqueued-limit-exceeded.yaml");
    assert_eq!(
        answer["outcome"],
        json!({
            "kind": "over_limit",
            "scope": {"key": "api_key", "value": "sk-oracle-key", "limit": 1000_i64},
            "enqueued": 950_i64,
        })
    );
    assert_eq!(
        answer["events"],
        json!([{
            "event": "reserve",
            "key": "batch_enqueued_tokens:api_key:sk-oracle-key",
            "args": [500_i64, 691_200_i64, 1000_i64],
        }])
    );
    assert_eq!(answer["memory"], json!({}));
}

#[test]
fn invalid_limit_warning_matches_the_pinned_answer() {
    let answer = run_case("invalid-limit-warning.yaml");
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
    let answer = run_case("partial-reserve-rollback.yaml");
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
            {
                "event": "reserve",
                "key": "batch_enqueued_tokens:api_key:sk-oracle-key",
                "args": [500_i64, 691_200_i64, 1000_i64],
            },
            {
                "event": "reserve",
                "key": "batch_enqueued_tokens:team:team-oracle",
                "args": [500_i64, 691_200_i64, 2000_i64],
            },
            {
                "event": "refund",
                "key": "batch_enqueued_tokens:api_key:sk-oracle-key",
                "args": [500_i64],
            },
        ])
    );
    assert_eq!(answer["memory"], json!({}));
}

#[test]
fn redis_fallback_memory_matches_the_pinned_answer() {
    let answer = run_case("redis-fallback-memory.yaml");
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
        json!([
            {
                "event": "reserve",
                "key": "batch_enqueued_tokens:api_key:sk-oracle-key",
                "args": [500_i64, 691_200_i64, 1000_i64],
            },
            {"event": "warning", "level": "warning"},
        ])
    );
    assert_eq!(
        answer["memory"],
        json!({
            "batch_enqueued_tokens:api_key:sk-oracle-key": 800_i64,
            "batch_enqueued_tokens:team:team-oracle": 600_i64,
        })
    );
}
