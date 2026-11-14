#![cfg(test)]

use crate::limiter_records::CacheCounterValue;

use super::{CounterValueError, counter_integer};

#[test]
fn counter_integer_reads_integers_text_and_bytes() {
    assert_eq!(
        counter_integer("reply[1]", &CacheCounterValue::Int(4)),
        Ok(4)
    );
    assert_eq!(
        counter_integer("reply[1]", &CacheCounterValue::Text("12".to_owned())),
        Ok(12)
    );
    assert_eq!(
        counter_integer("reply[1]", &CacheCounterValue::Text("-3".to_owned())),
        Ok(-3)
    );
    assert_eq!(
        counter_integer(
            "reply[1]",
            &CacheCounterValue::Bytes(b"1700000100".to_vec())
        ),
        Ok(1_700_000_100)
    );
}

#[test]
fn counter_integer_rejects_a_float() {
    assert_eq!(
        counter_integer("reply[1]", &CacheCounterValue::Float(2.5)),
        Err(CounterValueError::NotAnInteger {
            position: "reply[1]".to_owned(),
            value: CacheCounterValue::Float(2.5),
        })
    );
}

#[test]
fn counter_integer_rejects_text_that_is_not_a_number() {
    assert_eq!(
        counter_integer("reply[2]", &CacheCounterValue::Text("many".to_owned())),
        Err(CounterValueError::NotAnInteger {
            position: "reply[2]".to_owned(),
            value: CacheCounterValue::Text("many".to_owned()),
        })
    );
}

#[test]
fn counter_integer_rejects_bytes_that_are_not_a_number() {
    assert_eq!(
        counter_integer("reply[3]", &CacheCounterValue::Bytes(vec![0xff])),
        Err(CounterValueError::NotAnInteger {
            position: "reply[3]".to_owned(),
            value: CacheCounterValue::Bytes(vec![0xff]),
        })
    );
    assert_eq!(
        counter_integer("reply[3]", &CacheCounterValue::Bytes(b"soon".to_vec())),
        Err(CounterValueError::NotAnInteger {
            position: "reply[3]".to_owned(),
            value: CacheCounterValue::Bytes(b"soon".to_vec()),
        })
    );
}
