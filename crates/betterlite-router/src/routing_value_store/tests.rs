#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::error::Category;

use super::codec::parse_route_value;
use super::{
    CompareOutcome, PinValue, RouteIncrement, RouteKey, RouteValue, RoutingValueStore,
    SchedulerTuple, StoreError, StoreRead, StoreSettings,
};

struct TestClock(Arc<Mutex<f64>>);

fn clock() -> TestClock {
    TestClock(Arc::new(Mutex::new(f64::default())))
}

fn at(clock: &TestClock, now: f64) {
    *clock.0.lock().unwrap() = now;
}

fn store_with(capacity: Option<i64>, clock: &TestClock) -> RoutingValueStore {
    let source = Arc::clone(&clock.0);
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: capacity,
            default_ttl_seconds: None,
            max_size_per_item: None,
            loaded_max_size_per_item_kib: 1024,
        },
        Arc::new(move || *source.lock().unwrap()),
    )
    .unwrap()
}

fn route_key(raw: &str) -> RouteKey {
    RouteKey::parse(raw).unwrap()
}

fn counter(value: i64) -> RouteValue {
    RouteValue::Counter(value)
}

fn pin_with_serialized_len(total: usize) -> RouteValue {
    let empty = serde_json::to_vec(&RouteValue::Pin(PinValue {
        model_id: String::new(),
    }))
    .unwrap()
    .len();
    RouteValue::Pin(PinValue {
        model_id: "a".repeat(total.saturating_sub(empty)),
    })
}

#[rstest]
#[case::counter_round_trip("counter_round_trip")]
#[case::spend_round_trip("spend_round_trip")]
#[case::pin_round_trip("pin_round_trip")]
#[case::scheduler_tuple_round_trip("scheduler_tuple_round_trip")]
#[case::members_round_trip("members_round_trip")]
#[case::pipeline_writes_every_entry("pipeline_writes_every_entry")]
#[case::set_add_accumulates_members("set_add_accumulates_members")]
#[case::increment_counter_from_missing("increment_counter_from_missing")]
#[case::increment_counter_existing("increment_counter_existing")]
#[case::increment_spend_from_missing("increment_spend_from_missing")]
#[case::increment_spend_existing("increment_spend_existing")]
#[case::increment_keeps_live_expiry("increment_keeps_live_expiry")]
#[case::compare_and_set_first_writer("compare_and_set_first_writer")]
#[case::compare_and_set_keeps_winner("compare_and_set_keeps_winner")]
#[case::compare_and_set_swaps_matching("compare_and_set_swaps_matching")]
#[case::batch_get_keeps_order_and_duplicates("batch_get_keeps_order_and_duplicates")]
#[case::expiry_equality_is_readable("expiry_equality_is_readable")]
#[case::expired_read_is_missing("expired_read_is_missing")]
#[case::zero_ttl_is_readable_then_expires("zero_ttl_is_readable_then_expires")]
#[case::default_ttl_is_six_hundred_seconds("default_ttl_is_six_hundred_seconds")]
#[case::oversized_value_is_not_cached("oversized_value_is_not_cached")]
#[case::capacity_zero_stores_nothing("capacity_zero_stores_nothing")]
#[case::delete_removes_value("delete_removes_value")]
fn routing_value_store_behaviors(#[case] name: &str) {
    match name {
        "counter_round_trip" => counter_round_trip(),
        "spend_round_trip" => spend_round_trip(),
        "pin_round_trip" => pin_round_trip(),
        "scheduler_tuple_round_trip" => scheduler_tuple_round_trip(),
        "members_round_trip" => members_round_trip(),
        "pipeline_writes_every_entry" => pipeline_writes_every_entry(),
        "set_add_accumulates_members" => set_add_accumulates_members(),
        "increment_counter_from_missing" => increment_counter_from_missing(),
        "increment_counter_existing" => increment_counter_existing(),
        "increment_spend_from_missing" => increment_spend_from_missing(),
        "increment_spend_existing" => increment_spend_existing(),
        "increment_keeps_live_expiry" => increment_keeps_live_expiry(),
        "compare_and_set_first_writer" => compare_and_set_first_writer(),
        "compare_and_set_keeps_winner" => compare_and_set_keeps_winner(),
        "compare_and_set_swaps_matching" => compare_and_set_swaps_matching(),
        "batch_get_keeps_order_and_duplicates" => batch_get_keeps_order_and_duplicates(),
        "expiry_equality_is_readable" => expiry_equality_is_readable(),
        "expired_read_is_missing" => expired_read_is_missing(),
        "zero_ttl_is_readable_then_expires" => zero_ttl_is_readable_then_expires(),
        "default_ttl_is_six_hundred_seconds" => default_ttl_is_six_hundred_seconds(),
        "oversized_value_is_not_cached" => oversized_value_is_not_cached(),
        "capacity_zero_stores_nothing" => capacity_zero_stores_nothing(),
        "delete_removes_value" => delete_removes_value(),
        _ => assert!(name.is_empty()),
    }
}

#[rstest]
#[case::key_at_limit_is_accepted("key_at_limit_is_accepted")]
#[case::key_past_limit_is_rejected("key_past_limit_is_rejected")]
#[case::value_at_limit_is_stored("value_at_limit_is_stored")]
#[case::oversized_value_keeps_live_entries("oversized_value_keeps_live_entries")]
#[case::compare_and_set_refuses_an_oversized_value("compare_and_set_refuses_an_oversized_value")]
#[case::parser_at_limit_decodes("parser_at_limit_decodes")]
#[case::parser_past_limit_is_rejected("parser_past_limit_is_rejected")]
#[case::parser_malformed_keeps_its_cause("parser_malformed_keeps_its_cause")]
#[case::parser_unknown_variant_is_rejected("parser_unknown_variant_is_rejected")]
#[case::counter_overflow_is_reported("counter_overflow_is_reported")]
#[case::spend_overflow_is_reported("spend_overflow_is_reported")]
#[case::increment_mismatch_keeps_value("increment_mismatch_keeps_value")]
#[case::compare_and_set_mismatch_keeps_value("compare_and_set_mismatch_keeps_value")]
#[case::capacity_evicts_earliest_expiry("capacity_evicts_earliest_expiry")]
#[case::expired_entries_are_pruned_before_eviction("expired_entries_are_pruned_before_eviction")]
#[case::overwrite_at_capacity_resets_expiry("overwrite_at_capacity_resets_expiry")]
fn routing_value_store_boundaries(#[case] name: &str) {
    match name {
        "key_at_limit_is_accepted" => key_at_limit_is_accepted(),
        "key_past_limit_is_rejected" => key_past_limit_is_rejected(),
        "value_at_limit_is_stored" => value_at_limit_is_stored(),
        "oversized_value_keeps_live_entries" => oversized_value_keeps_live_entries(),
        "compare_and_set_refuses_an_oversized_value" => {
            compare_and_set_refuses_an_oversized_value();
        }
        "parser_at_limit_decodes" => parser_at_limit_decodes(),
        "parser_past_limit_is_rejected" => parser_past_limit_is_rejected(),
        "parser_malformed_keeps_its_cause" => parser_malformed_keeps_its_cause(),
        "parser_unknown_variant_is_rejected" => parser_unknown_variant_is_rejected(),
        "counter_overflow_is_reported" => counter_overflow_is_reported(),
        "spend_overflow_is_reported" => spend_overflow_is_reported(),
        "increment_mismatch_keeps_value" => increment_mismatch_keeps_value(),
        "compare_and_set_mismatch_keeps_value" => compare_and_set_mismatch_keeps_value(),
        "capacity_evicts_earliest_expiry" => capacity_evicts_earliest_expiry(),
        "expired_entries_are_pruned_before_eviction" => {
            expired_entries_are_pruned_before_eviction();
        }
        "overwrite_at_capacity_resets_expiry" => overwrite_at_capacity_resets_expiry(),
        _ => assert!(name.is_empty()),
    }
}

fn counter_round_trip() {
    let store = store_with(None, &clock());
    let key = route_key("global_router:a:tpm:00-00");
    store.set(&key, &counter(7), None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(7)));
}

fn spend_round_trip() {
    let store = store_with(None, &clock());
    let key = route_key("global_router:a:tpm:00-00");
    store
        .set(&key, &RouteValue::Spend(Decimal::new(1225, 2)), None)
        .unwrap();
    assert_eq!(
        store.get(&key).unwrap(),
        StoreRead::Value(RouteValue::Spend(Decimal::new(1225, 2)))
    );
}

fn pin_round_trip() {
    let store = store_with(None, &clock());
    let key = route_key("affinity:user");
    let value = RouteValue::Pin(PinValue {
        model_id: "deployment-1".to_owned(),
    });
    store.set(&key, &value, None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
}

fn scheduler_tuple_round_trip() {
    let store = store_with(None, &clock());
    let key = route_key("scheduler:queue:model");
    let value = RouteValue::SchedulerTuple(SchedulerTuple {
        priority: 3,
        request_id: "req-1".to_owned(),
    });
    store.set(&key, &value, None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
}

fn members_round_trip() {
    let store = store_with(None, &clock());
    let key = route_key("dynamic_rate:00-00:model");
    let value = RouteValue::Members(["a".to_owned(), "b".to_owned()].into_iter().collect());
    store.set(&key, &value, None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
}

fn pipeline_writes_every_entry() {
    let store = store_with(None, &clock());
    let first = route_key("first");
    let second = route_key("second");
    store
        .set_pipeline(
            &[
                (first.clone(), counter(1)),
                (second.clone(), counter(2)),
                (first.clone(), counter(3)),
            ],
            None,
        )
        .unwrap();
    assert_eq!(store.get(&first).unwrap(), StoreRead::Value(counter(3)));
    assert_eq!(store.get(&second).unwrap(), StoreRead::Value(counter(2)));
}

fn set_add_accumulates_members() {
    let store = store_with(None, &clock());
    let key = route_key("dynamic_rate:00-00:model");
    let first = vec!["a".to_owned(), "b".to_owned()];
    let second = vec!["b".to_owned(), "c".to_owned()];
    assert_eq!(store.set_add(&key, &first, None).unwrap(), first);
    assert_eq!(store.set_add(&key, &second, None).unwrap(), second);
    let expected = RouteValue::Members(
        ["a".to_owned(), "b".to_owned(), "c".to_owned()]
            .into_iter()
            .collect(),
    );
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(expected));
}

fn increment_counter_from_missing() {
    let store = store_with(None, &clock());
    let key = route_key("in-flight");
    let value = store
        .increment(&key, &RouteIncrement::Counter(3), None)
        .unwrap();
    assert_eq!(value, counter(3));
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(3)));
}

fn increment_counter_existing() {
    let store = store_with(None, &clock());
    let key = route_key("in-flight");
    store.set(&key, &counter(2), None).unwrap();
    let value = store
        .increment(&key, &RouteIncrement::Counter(5), None)
        .unwrap();
    assert_eq!(value, counter(7));
}

fn increment_spend_from_missing() {
    let store = store_with(None, &clock());
    let key = route_key("spend");
    let value = store
        .increment(&key, &RouteIncrement::Spend(Decimal::new(25, 2)), None)
        .unwrap();
    assert_eq!(value, RouteValue::Spend(Decimal::new(25, 2)));
}

fn increment_spend_existing() {
    let store = store_with(None, &clock());
    let key = route_key("spend");
    store
        .set(&key, &RouteValue::Spend(Decimal::new(25, 2)), None)
        .unwrap();
    let value = store
        .increment(&key, &RouteIncrement::Spend(Decimal::new(5, 1)), None)
        .unwrap();
    assert_eq!(value, RouteValue::Spend(Decimal::new(75, 2)));
}

fn increment_keeps_live_expiry() {
    let clock = clock();
    let store = store_with(None, &clock);
    let key = route_key("counter");
    at(&clock, 0.0_f64);
    store.set(&key, &counter(2), Some(10.0_f64)).unwrap();
    at(&clock, 5.0_f64);
    let value = store
        .increment(&key, &RouteIncrement::Counter(3), Some(100.0_f64))
        .unwrap();
    assert_eq!(value, counter(5));
    at(&clock, 10.0_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(5)));
    at(&clock, 10.5_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
}

fn compare_and_set_first_writer() {
    let store = store_with(None, &clock());
    let key = route_key("affinity:user");
    let first = RouteValue::Pin(PinValue {
        model_id: "first".to_owned(),
    });
    let outcome = store
        .compare_and_set(&key, &StoreRead::Missing, &first, None)
        .unwrap();
    assert_eq!(
        outcome,
        CompareOutcome {
            swapped: true,
            current: StoreRead::Value(first.clone()),
        }
    );
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(first));
}

fn compare_and_set_keeps_winner() {
    let store = store_with(None, &clock());
    let key = route_key("affinity:user");
    let first = RouteValue::Pin(PinValue {
        model_id: "first".to_owned(),
    });
    let second = RouteValue::Pin(PinValue {
        model_id: "second".to_owned(),
    });
    store
        .compare_and_set(&key, &StoreRead::Missing, &first, None)
        .unwrap();
    let outcome = store
        .compare_and_set(&key, &StoreRead::Value(second.clone()), &second, None)
        .unwrap();
    assert_eq!(
        outcome,
        CompareOutcome {
            swapped: false,
            current: StoreRead::Value(first.clone()),
        }
    );
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(first));
}

fn compare_and_set_swaps_matching() {
    let store = store_with(None, &clock());
    let key = route_key("affinity:user");
    let first = RouteValue::Pin(PinValue {
        model_id: "first".to_owned(),
    });
    let second = RouteValue::Pin(PinValue {
        model_id: "second".to_owned(),
    });
    store.set(&key, &first, None).unwrap();
    let outcome = store
        .compare_and_set(&key, &StoreRead::Value(first), &second, None)
        .unwrap();
    assert_eq!(
        outcome,
        CompareOutcome {
            swapped: true,
            current: StoreRead::Value(second.clone()),
        }
    );
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(second));
}

fn batch_get_keeps_order_and_duplicates() {
    let clock = clock();
    let store = store_with(None, &clock);
    let first = route_key("first");
    let second = route_key("second");
    let expired = route_key("expired");
    store.set(&first, &counter(1), None).unwrap();
    store
        .set(&second, &RouteValue::Spend(Decimal::new(25, 2)), None)
        .unwrap();
    store.set(&expired, &counter(9), Some(5.0_f64)).unwrap();
    at(&clock, 6.0_f64);
    let missing = route_key("missing");
    let read = store
        .batch_get(&[first.clone(), missing, second, expired, first])
        .unwrap();
    assert_eq!(
        read,
        vec![
            StoreRead::Value(counter(1)),
            StoreRead::Missing,
            StoreRead::Value(RouteValue::Spend(Decimal::new(25, 2))),
            StoreRead::Missing,
            StoreRead::Value(counter(1)),
        ]
    );
}

fn expiry_equality_is_readable() {
    let clock = clock();
    let store = store_with(None, &clock);
    let key = route_key("counter");
    at(&clock, 0.0_f64);
    store.set(&key, &counter(1), Some(10.0_f64)).unwrap();
    at(&clock, 10.0_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(1)));
    at(&clock, 10.5_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
}

fn expired_read_is_missing() {
    let clock = clock();
    let store = store_with(None, &clock);
    let key = route_key("counter");
    at(&clock, 0.0_f64);
    store.set(&key, &counter(1), Some(10.0_f64)).unwrap();
    at(&clock, 11.0_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
}

fn zero_ttl_is_readable_then_expires() {
    let clock = clock();
    let store = store_with(None, &clock);
    let key = route_key("counter");
    at(&clock, 0.0_f64);
    store.set(&key, &counter(1), Some(0.0_f64)).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(1)));
    at(&clock, 0.5_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
}

fn default_ttl_is_six_hundred_seconds() {
    let clock = clock();
    let store = store_with(None, &clock);
    let key = route_key("counter");
    at(&clock, 0.0_f64);
    store.set(&key, &counter(1), None).unwrap();
    at(&clock, 599.5_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(1)));
    at(&clock, 600.0_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(1)));
    at(&clock, 600.5_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
}

fn oversized_value_is_not_cached() {
    let store = store_with(None, &clock());
    let key = route_key("huge");
    let value = pin_with_serialized_len(1_048_577);
    store.set(&key, &value, None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
}

fn capacity_zero_stores_nothing() {
    let store = store_with(Some(0), &clock());
    let key = route_key("counter");
    store.set(&key, &counter(1), None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
}

fn delete_removes_value() {
    let store = store_with(None, &clock());
    let key = route_key("counter");
    store.set(&key, &counter(1), None).unwrap();
    store.delete(&key);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
}

// the pinned limits are literals here, so a mutated bound constant cannot move the boundary along with its test
fn key_at_limit_is_accepted() {
    let raw = "k".repeat(1_024);
    let key = RouteKey::parse(&raw).unwrap();
    assert_eq!(key.as_ref().len(), 1_024);
    let store = store_with(None, &clock());
    store.set(&key, &counter(1), None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(1)));
}

fn key_past_limit_is_rejected() {
    let error = RouteKey::parse(&"k".repeat(1_025)).unwrap_err();
    assert!(matches!(
        error,
        StoreError::KeyTooLong { bytes, limit }
            if bytes == 1_025 && limit == 1_024
    ));
}

fn value_at_limit_is_stored() {
    let store = store_with(None, &clock());
    let key = route_key("at-limit");
    let value = pin_with_serialized_len(1_048_576);
    store.set(&key, &value, None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
}

fn oversized_value_keeps_live_entries() {
    let store = store_with(Some(1), &clock());
    let kept = route_key("kept");
    let huge = route_key("huge");
    store.set(&kept, &counter(1), None).unwrap();
    let value = pin_with_serialized_len(1_048_577);
    store.set(&huge, &value, None).unwrap();
    assert_eq!(store.get(&kept).unwrap(), StoreRead::Value(counter(1)));
    assert_eq!(store.get(&huge).unwrap(), StoreRead::Missing);
}

fn compare_and_set_refuses_an_oversized_value() {
    let store = store_with(None, &clock());
    let key = route_key("counter");
    store.set(&key, &counter(1), None).unwrap();
    let value = pin_with_serialized_len(1_048_577);
    let outcome = store
        .compare_and_set(&key, &StoreRead::Value(counter(1)), &value, None)
        .unwrap();
    assert_eq!(
        outcome,
        CompareOutcome {
            swapped: false,
            current: StoreRead::Value(counter(1)),
        }
    );
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(1)));
}

fn parser_at_limit_decodes() {
    let value = pin_with_serialized_len(1_048_576);
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(bytes.len(), 1_048_576);
    assert_eq!(parse_route_value(&bytes).unwrap(), value);
}

fn parser_past_limit_is_rejected() {
    let bytes = serde_json::to_vec(&pin_with_serialized_len(1_048_577)).unwrap();
    assert!(matches!(
        parse_route_value(&bytes),
        Err(StoreError::ValueTooLarge { bytes: reported, limit })
            if reported == 1_048_577 && limit == 1_048_576
    ));
}

fn parser_malformed_keeps_its_cause() {
    let outcome = parse_route_value(b"{");
    if let Err(StoreError::InvalidValue { source }) = &outcome {
        assert_eq!(source.classify(), Category::Eof);
    } else {
        panic!("the malformed value must not decode");
    }
}

fn parser_unknown_variant_is_rejected() {
    let outcome = parse_route_value(br#"{"unknown":1}"#);
    if let Err(StoreError::InvalidValue { source }) = &outcome {
        assert_eq!(source.classify(), Category::Data);
    } else {
        panic!("the unknown variant must not decode");
    }
}

fn counter_overflow_is_reported() {
    let store = store_with(None, &clock());
    let key = route_key("overflow");
    store.set(&key, &counter(i64::MAX), None).unwrap();
    assert!(matches!(
        store.increment(&key, &RouteIncrement::Counter(1), None),
        Err(StoreError::CounterOverflow { key: reported }) if reported == "overflow"
    ));
    assert_eq!(
        store.get(&key).unwrap(),
        StoreRead::Value(counter(i64::MAX))
    );
}

fn spend_overflow_is_reported() {
    let store = store_with(None, &clock());
    let key = route_key("overflow");
    store
        .set(&key, &RouteValue::Spend(Decimal::MAX), None)
        .unwrap();
    assert!(matches!(
        store.increment(&key, &RouteIncrement::Spend(Decimal::ONE), None),
        Err(StoreError::SpendOverflow { key: reported }) if reported == "overflow"
    ));
    assert_eq!(
        store.get(&key).unwrap(),
        StoreRead::Value(RouteValue::Spend(Decimal::MAX))
    );
}

fn increment_mismatch_keeps_value() {
    let store = store_with(None, &clock());
    let key = route_key("pin");
    let value = RouteValue::Pin(PinValue {
        model_id: "deployment-1".to_owned(),
    });
    store.set(&key, &value, None).unwrap();
    assert!(matches!(
        store.increment(&key, &RouteIncrement::Counter(1), None),
        Err(StoreError::NotIncrementable { key: reported }) if reported == "pin"
    ));
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
}

fn compare_and_set_mismatch_keeps_value() {
    let store = store_with(None, &clock());
    let key = route_key("counter");
    store.set(&key, &counter(1), None).unwrap();
    let outcome = store
        .compare_and_set(&key, &StoreRead::Value(counter(2)), &counter(3), None)
        .unwrap();
    assert_eq!(
        outcome,
        CompareOutcome {
            swapped: false,
            current: StoreRead::Value(counter(1)),
        }
    );
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(1)));
}

fn capacity_evicts_earliest_expiry() {
    let clock = clock();
    let store = store_with(Some(2), &clock);
    let first = route_key("first");
    let second = route_key("second");
    let third = route_key("third");
    at(&clock, 0.0_f64);
    store.set(&first, &counter(1), Some(10.0_f64)).unwrap();
    store.set(&second, &counter(2), Some(20.0_f64)).unwrap();
    store.set(&third, &counter(3), Some(30.0_f64)).unwrap();
    assert_eq!(store.get(&first).unwrap(), StoreRead::Missing);
    assert_eq!(store.get(&second).unwrap(), StoreRead::Value(counter(2)));
    assert_eq!(store.get(&third).unwrap(), StoreRead::Value(counter(3)));
}

fn expired_entries_are_pruned_before_eviction() {
    let clock = clock();
    let store = store_with(Some(2), &clock);
    let first = route_key("first");
    let second = route_key("second");
    let third = route_key("third");
    at(&clock, 0.0_f64);
    store.set(&first, &counter(1), Some(5.0_f64)).unwrap();
    store.set(&second, &counter(2), Some(10.0_f64)).unwrap();
    at(&clock, 6.0_f64);
    store.set(&third, &counter(3), Some(10.0_f64)).unwrap();
    assert_eq!(store.get(&first).unwrap(), StoreRead::Missing);
    assert_eq!(store.get(&second).unwrap(), StoreRead::Value(counter(2)));
    assert_eq!(store.get(&third).unwrap(), StoreRead::Value(counter(3)));
}

fn overwrite_at_capacity_resets_expiry() {
    let clock = clock();
    let store = store_with(Some(2), &clock);
    let first = route_key("first");
    let second = route_key("second");
    at(&clock, 0.0_f64);
    store.set(&first, &counter(1), Some(10.0_f64)).unwrap();
    store.set(&second, &counter(2), Some(20.0_f64)).unwrap();
    at(&clock, 5.0_f64);
    store.set(&first, &counter(9), Some(100.0_f64)).unwrap();
    at(&clock, 11.0_f64);
    assert_eq!(store.get(&first).unwrap(), StoreRead::Value(counter(9)));
    at(&clock, 21.0_f64);
    assert_eq!(store.get(&second).unwrap(), StoreRead::Missing);
    at(&clock, 105.0_f64);
    assert_eq!(store.get(&first).unwrap(), StoreRead::Value(counter(9)));
    at(&clock, 106.0_f64);
    assert_eq!(store.get(&first).unwrap(), StoreRead::Missing);
}

#[rstest]
fn replays_the_route_value_corpus(#[files("fuzz/corpus/route_value/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let outcome = parse_route_value(&bytes);
    if name.contains("invalid") {
        assert!(outcome.is_err(), "{name} must not decode");
    } else {
        assert!(outcome.is_ok(), "{name} must decode");
    }
}
