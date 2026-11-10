use std::time::Duration;

use rust_decimal::Decimal;

use super::support::queued;
use crate::budget_increment::error::IncrementError;
use crate::budget_increment::queue::QueuedIncrement;
use crate::budget_increment::redis_cache_window::pipeline_operations;

#[test]
fn pipeline_operations_carry_the_key_cost_and_integer_ttl() {
    let operations = vec![
        queued("spend:a", Decimal::new(25, 2), 30),
        queued("spend:b", Decimal::new(15, 1), 5),
    ];
    let increments = pipeline_operations(&operations).unwrap();
    assert_eq!(increments.len(), 2);
    assert_eq!(increments[0].key, "spend:a");
    assert_eq!(increments[0].increment_value.to_bits(), 0.25_f64.to_bits());
    assert_eq!(increments[0].ttl, Some(30));
    assert_eq!(increments[1].key, "spend:b");
    assert_eq!(increments[1].increment_value.to_bits(), 1.5_f64.to_bits());
    assert_eq!(increments[1].ttl, Some(5));
}

#[test]
fn pipeline_operations_truncate_a_fractional_ttl() {
    let operation = QueuedIncrement {
        key: "spend:a".to_owned(),
        increment_value: Decimal::ONE,
        ttl: Duration::from_millis(30_900),
    };
    let increments = pipeline_operations(&[operation]).unwrap();
    assert_eq!(increments[0].ttl, Some(30));
}

#[test]
fn pipeline_operations_reject_a_ttl_past_i64() {
    let operation = QueuedIncrement {
        key: "spend:a".to_owned(),
        increment_value: Decimal::ONE,
        ttl: Duration::from_secs(u64::MAX),
    };
    let error = pipeline_operations(&[operation]).unwrap_err();
    assert!(matches!(
        error,
        IncrementError::TtlOutOfRange { seconds } if seconds == u64::MAX
    ));
}

#[test]
fn pipeline_operations_accept_an_empty_batch() {
    assert!(pipeline_operations(&[]).unwrap().is_empty());
}
