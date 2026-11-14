#![cfg(test)]

use serde_json::{Value, json};

use crate::helpers::Helper as _;

use super::Op;

const RECORD_KEY: &str = "batch_enqueued_token_reservation:batch-1";
const KEY_COUNTER: &str = "batch_enqueued_tokens:api_key:sk-oracle-key";

fn run(input: &Value) -> Value {
    Op::decode_and_run(input).expect("the helper runs")
}

fn scope() -> Value {
    json!({"key": "api_key", "value": "sk-oracle-key", "limit": 1000_i64})
}

fn record() -> Value {
    json!({
        "tokens": 500_i64,
        "scopes": [scope()],
        "backend": "redis",
        "owner": "",
        "reserved_at_monotonic": 12.5_f64,
    })
}

#[test]
fn terminal_post_call_refunds_the_stored_record() {
    let answer = run(&json!({
        "plan": {
            "op": "post_call",
            "batch_id": "batch-1",
            "status": "completed",
            "pending": {
                "tokens": 500_i64,
                "scopes": [scope()],
                "reserved_at_monotonic": 12.5_f64,
            },
        },
        "redis": {
            "save": [{"kind": "done"}],
            "pop": [{"kind": "record", "record": record()}],
            "refund": [{"kind": "done"}],
        },
        "memory": {},
        "now": 20.0_f64,
    }));
    assert_eq!(
        answer["outcome"],
        json!({
            "kind": "post_call",
            "batch_id": "batch-1",
            "saved": true,
            "refunded": true,
        })
    );
    assert_eq!(
        answer["events"],
        json!([
            {"event": "save", "key": RECORD_KEY, "record": record(), "args": [691_192_i64]},
            {"event": "pop", "key": RECORD_KEY, "args": [691_200_i64]},
            {"event": "refund", "key": KEY_COUNTER, "args": [500_i64]},
        ])
    );
    assert_eq!(answer["memory"], json!({}));
}

#[test]
fn tombstoned_pop_drops_the_local_copy() {
    let stored = r#"{"tokens":500,"scopes":[],"reserved_at_monotonic":12.5}"#;
    let answer = run(&json!({
        "plan": {"op": "pop", "batch_id": "batch-1"},
        "redis": {"pop": [{"kind": "tombstone"}]},
        "memory": {"records": {RECORD_KEY: stored}, "read": [RECORD_KEY]},
    }));
    assert_eq!(
        answer["outcome"],
        json!({"kind": "popped", "reservation": null})
    );
    assert_eq!(
        answer["events"],
        json!([
            {"event": "pop", "key": RECORD_KEY, "args": [691_200_i64]},
            {"event": "local_delete", "key": RECORD_KEY},
        ])
    );
    assert_eq!(answer["memory"], json!({RECORD_KEY: null}));
}
