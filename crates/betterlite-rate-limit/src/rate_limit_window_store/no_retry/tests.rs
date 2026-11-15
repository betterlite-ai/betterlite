#![cfg(test)]

use super::read_group_values_once;
use crate::limiter_records::{CacheCounterValue, ScriptReply};
use crate::rate_limit_window_store::WindowStoreError;
use crate::rate_limit_window_store::batch_counter_read_script::BATCH_COUNTER_READ_SCRIPT;
use crate::rate_limit_window_store::tests::support::{FakeScript, key_list};

fn groups() -> Vec<(String, Vec<String>)> {
    vec![
        ("slot_1".to_owned(), key_list(&["alpha"])),
        ("slot_2".to_owned(), key_list(&["beta"])),
        ("slot_3".to_owned(), key_list(&["gamma"])),
    ]
}

#[tokio::test]
async fn every_group_returns_its_own_values_in_order() {
    let script = FakeScript::new(
        BATCH_COUNTER_READ_SCRIPT,
        vec![
            ScriptReply::Array(vec![ScriptReply::Int(1)]),
            ScriptReply::Array(vec![ScriptReply::Int(2)]),
        ],
    );
    let values = read_group_values_once(script.as_ref(), &groups()[..2])
        .await
        .expect("the grouped read");
    assert_eq!(
        values,
        vec![
            vec![Some(CacheCounterValue::Int(1))],
            vec![Some(CacheCounterValue::Int(2))],
        ]
    );
    assert_eq!(script.calls().len(), 2);
}

#[tokio::test]
async fn a_failing_group_is_called_once_and_the_error_comes_back() {
    let script = FakeScript::new(
        BATCH_COUNTER_READ_SCRIPT,
        vec![ScriptReply::Array(vec![ScriptReply::Int(1)])],
    )
    .failing_after(2);
    let error = read_group_values_once(script.as_ref(), &groups())
        .await
        .expect_err("the second group fails");
    assert!(matches!(error, WindowStoreError::Script(_)));
    assert_eq!(script.calls().len(), 2);
}

#[tokio::test]
async fn a_failure_stops_the_groups_after_it() {
    let script = FakeScript::new(BATCH_COUNTER_READ_SCRIPT, Vec::new()).failing_after(1);
    read_group_values_once(script.as_ref(), &groups())
        .await
        .expect_err("the first group fails");
    assert_eq!(script.calls().len(), 1);
}
