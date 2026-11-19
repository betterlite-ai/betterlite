#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::{CacheCounterValue, ScriptReply};
use crate::rate_limit_window_store::ScriptHandle;
use crate::rate_limit_window_store::batch_rate_limiter_script::window_script_args;
use crate::windowed_admission::WindowedAdmissionError;
use crate::windowed_admission::tests::support::{
    HANDLER_WINDOW_SIZE, LogCapture, NOW, PlannedBatch, PlannedScript, ReplyPlan, ScriptCall,
    harness, int_plan, key_list, mixed_plan, text,
};

fn group_keys(tag: &str) -> Vec<String> {
    key_list(&[&format!("{{{tag}}}:window"), &format!("{{{tag}}}:requests")])
}

fn two_groups() -> Vec<String> {
    let mut keys = group_keys("api-key:a");
    keys.extend(group_keys("api-key:b"));
    keys
}

fn refund_args(window_start: &str) -> Vec<RedisScalar> {
    vec![
        RedisScalar::Text(window_start.to_owned()),
        RedisScalar::Text("-1".to_owned()),
        RedisScalar::Text("0".to_owned()),
    ]
}

#[tokio::test]
async fn no_rate_limiter_script_means_no_values_and_no_calls() {
    let mut harness = harness();
    harness.script_enabled = false;
    let keys = two_groups();
    let limiter = harness.limiter();
    let values = limiter
        .execute_redis_batch_rate_limiter_script(&keys, NOW)
        .await
        .unwrap();
    assert!(values.is_empty());
    assert!(harness.script.calls().is_empty());
    assert!(harness.cache.reads().is_empty());
}

#[tokio::test]
async fn each_hash_tag_group_runs_once_in_order() {
    let mut harness = harness();
    harness.cache.as_cluster();
    harness.script = PlannedScript::new(vec![
        mixed_plan(&[text("1700000000"), Some(CacheCounterValue::Int(1))]),
        mixed_plan(&[text("1700000000"), Some(CacheCounterValue::Int(2))]),
    ]);
    let keys = two_groups();
    let limiter = harness.limiter();
    let values = limiter
        .execute_redis_batch_rate_limiter_script(&keys, NOW)
        .await
        .unwrap();
    assert_eq!(
        values,
        vec![
            CacheCounterValue::Text("1700000000".to_owned()),
            CacheCounterValue::Int(1),
            CacheCounterValue::Text("1700000000".to_owned()),
            CacheCounterValue::Int(2),
        ]
    );
    assert_eq!(
        harness.script.calls(),
        vec![
            ScriptCall {
                keys: group_keys("api-key:a"),
                args: window_script_args(NOW, HANDLER_WINDOW_SIZE),
            },
            ScriptCall {
                keys: group_keys("api-key:b"),
                args: window_script_args(NOW, HANDLER_WINDOW_SIZE),
            },
        ]
    );
}

#[tokio::test]
async fn a_failed_group_falls_back_to_the_sliding_window() {
    let mut harness = harness();
    harness.cache.as_cluster();
    harness.script = PlannedScript::new(vec![
        mixed_plan(&[text("1700000000"), Some(CacheCounterValue::Int(7))]),
        ReplyPlan::Failure,
    ]);
    let capture = LogCapture::start();
    let keys = two_groups();
    let limiter = harness.limiter();
    let values = limiter
        .execute_redis_batch_rate_limiter_script(&keys, NOW)
        .await
        .unwrap();
    assert_eq!(
        values,
        vec![
            CacheCounterValue::Text("1700000000".to_owned()),
            CacheCounterValue::Int(7),
            CacheCounterValue::Text(NOW.to_string()),
            CacheCounterValue::Int(1),
        ]
    );
    assert!(harness.refunds.calls().is_empty());
    let logged = capture.text();
    assert!(logged.contains("Redis Lua script failed for hash tag slot_12288"));
    assert!(!logged.contains("slot_99"));
}

#[tokio::test]
async fn a_failed_group_with_fail_closed_refunds_the_earlier_groups_and_rejects() {
    let mut harness = harness();
    harness.cache.as_cluster();
    harness.fail_closed = FailClosedPolicy { enabled: true };
    harness.script = PlannedScript::new(vec![
        mixed_plan(&[text("1700000000"), Some(CacheCounterValue::Int(7))]),
        ReplyPlan::Failure,
    ]);
    let capture = LogCapture::start();
    let keys = two_groups();
    let limiter = harness.limiter();
    let error = limiter
        .execute_redis_batch_rate_limiter_script(&keys, NOW)
        .await
        .unwrap_err();
    match error {
        WindowedAdmissionError::Unverifiable(unverifiable) => {
            assert_eq!(unverifiable.status_code, 503);
        }
        other => panic!("expected an unverifiable error, got {other}"),
    }
    assert_eq!(
        harness.refunds.calls(),
        vec![ScriptCall {
            keys: group_keys("api-key:a"),
            args: refund_args("1700000000"),
        }]
    );
    assert!(capture.text().contains("batch_rate_limiter_script"));
}

#[tokio::test]
async fn fail_closed_refunds_the_later_pipelined_groups() {
    let mut harness = harness();
    harness.cache.as_cluster();
    harness.fail_closed = FailClosedPolicy { enabled: true };
    harness.batch_enabled = true;
    harness.batch = PlannedBatch::new(vec![
        mixed_plan(&[text("1700000000"), Some(CacheCounterValue::Int(7))]),
        ReplyPlan::Failure,
        mixed_plan(&[text("1700000000"), Some(CacheCounterValue::Int(9))]),
    ]);
    let mut keys = group_keys("api-key:a");
    keys.extend(group_keys("api-key:b"));
    keys.extend(group_keys("api-key:c"));
    let limiter = harness.limiter();
    let _error = limiter
        .execute_redis_batch_rate_limiter_script(&keys, NOW)
        .await
        .unwrap_err();
    assert_eq!(
        harness.refunds.calls(),
        vec![
            ScriptCall {
                keys: group_keys("api-key:a"),
                args: refund_args("1700000000"),
            },
            ScriptCall {
                keys: group_keys("api-key:c"),
                args: refund_args("1700000000"),
            },
        ]
    );
}

#[tokio::test]
async fn the_refund_of_later_groups_skips_missing_handles_and_broken_replies() {
    let harness = harness();
    let groups = vec![
        ("slot_1".to_owned(), group_keys("api-key:a")),
        ("slot_2".to_owned(), group_keys("api-key:b")),
        ("slot_3".to_owned(), group_keys("api-key:c")),
        ("slot_4".to_owned(), group_keys("api-key:d")),
    ];
    let mut handles: Vec<Option<ScriptHandle>> = vec![
        None,
        Some(Box::pin(async { Err(Arc::new(BatchError::Cancelled)) })),
        Some(Box::pin(async { Ok(ScriptReply::Int(1)) })),
        Some(Box::pin(async {
            Ok(ScriptReply::Array(vec![
                ScriptReply::Text("1700000000".to_owned()),
                ScriptReply::Int(3),
            ]))
        })),
    ];
    let limiter = harness.limiter();
    limiter
        .refund_later_pipelined_groups(&groups, &mut handles)
        .await
        .unwrap();
    assert_eq!(
        harness.refunds.calls(),
        vec![ScriptCall {
            keys: group_keys("api-key:d"),
            args: refund_args("1700000000"),
        }]
    );
}

#[tokio::test]
async fn a_direct_group_runs_the_plain_script_with_the_handler_window() {
    let mut harness = harness();
    harness.script = PlannedScript::new(vec![int_plan(&[1700000000, 4])]);
    let keys = two_groups();
    let limiter = harness.limiter();
    let values = limiter
        .execute_redis_batch_rate_limiter_script(&keys, NOW)
        .await
        .unwrap();
    assert_eq!(
        values,
        vec![
            CacheCounterValue::Int(1700000000),
            CacheCounterValue::Int(4)
        ]
    );
    assert_eq!(
        harness.script.calls(),
        vec![ScriptCall {
            keys,
            args: window_script_args(NOW, HANDLER_WINDOW_SIZE),
        }]
    );
}
