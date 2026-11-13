#![cfg(test)]

use super::{BATCH_COUNTER_READ_SCRIPT, counter_read_values_from_reply};
use crate::limiter_records::{CacheCounterValue, CounterReplyError, ScriptReply, ScriptReplyKind};

#[test]
fn the_script_reads_every_key_with_one_mget() {
    assert_eq!(
        BATCH_COUNTER_READ_SCRIPT,
        "return redis.call('MGET', unpack(KEYS))"
    );
}

#[test]
fn the_read_result_preserves_nil_for_a_missing_key_and_zero_for_a_zero_counter() {
    let values = counter_read_values_from_reply(&ScriptReply::Array(vec![
        ScriptReply::Nil,
        ScriptReply::Int(0),
        ScriptReply::Text("3".to_owned()),
        ScriptReply::Bytes(b"4".to_vec()),
        ScriptReply::Float(5.5),
    ]))
    .expect("the counter read");
    assert_eq!(
        values,
        vec![
            None,
            Some(CacheCounterValue::Int(0)),
            Some(CacheCounterValue::Text("3".to_owned())),
            Some(CacheCounterValue::Bytes(b"4".to_vec())),
            Some(CacheCounterValue::Float(5.5)),
        ]
    );
}

#[test]
fn a_reply_that_is_not_an_array_is_an_error() {
    let error = counter_read_values_from_reply(&ScriptReply::Int(3)).expect_err("a scalar reply");
    assert_eq!(
        error,
        CounterReplyError::NotAnArray {
            kind: ScriptReplyKind::Int
        }
    );
}

#[test]
fn a_nested_array_reply_is_an_error() {
    let error =
        counter_read_values_from_reply(&ScriptReply::Array(vec![ScriptReply::Array(Vec::new())]))
            .expect_err("a nested array reply");
    assert_eq!(
        error,
        CounterReplyError::UnsupportedElement {
            kind: ScriptReplyKind::Array
        }
    );
}
