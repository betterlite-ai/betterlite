use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::error::Category;

use crate::routing_value_store::codec::parse_route_value;
use crate::routing_value_store::{
    CompareOutcome, PinValue, RouteIncrement, RouteKey, RouteValue, StoreError, StoreRead,
};

use super::{at, clock, counter, pin_with_serialized_len, route_key, store_with};

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
// the pinned limits are literals here (0x0010_0000 is 1 MiB), so a mutated bound constant cannot move the boundary along with its test
fn routing_value_store_boundaries(#[case] name: &str) {
    match name {
        "key_at_limit_is_accepted" => {
            let raw = "k".repeat(1_024);
            let key = RouteKey::parse(&raw).unwrap();
            assert_eq!(key.as_ref().len(), 1_024);
            let store = store_with(None, &clock());
            store.set(&key, &counter(1), None).unwrap();
            assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(1)));
        }
        "key_past_limit_is_rejected" => {
            let error = RouteKey::parse(&"k".repeat(1_025)).unwrap_err();
            assert!(matches!(
                error,
                StoreError::KeyTooLong { bytes, limit }
                    if bytes == 1_025 && limit == 1_024
            ));
        }
        "value_at_limit_is_stored" => {
            let store = store_with(None, &clock());
            let key = route_key("at-limit");
            let value = pin_with_serialized_len(0x0010_0000);
            store.set(&key, &value, None).unwrap();
            assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
        }
        "oversized_value_keeps_live_entries" => {
            let store = store_with(Some(1), &clock());
            let kept = route_key("kept");
            let huge = route_key("huge");
            store.set(&kept, &counter(1), None).unwrap();
            let value = pin_with_serialized_len(0x0010_0001);
            store.set(&huge, &value, None).unwrap();
            assert_eq!(store.get(&kept).unwrap(), StoreRead::Value(counter(1)));
            assert_eq!(store.get(&huge).unwrap(), StoreRead::Missing);
        }
        "compare_and_set_refuses_an_oversized_value" => {
            let store = store_with(None, &clock());
            let key = route_key("counter");
            store.set(&key, &counter(1), None).unwrap();
            let value = pin_with_serialized_len(0x0010_0001);
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
        "parser_at_limit_decodes" => {
            let value = pin_with_serialized_len(0x0010_0000);
            let bytes = serde_json::to_vec(&value).unwrap();
            assert_eq!(bytes.len(), 0x0010_0000);
            assert_eq!(parse_route_value(&bytes).unwrap(), value);
        }
        "parser_past_limit_is_rejected" => {
            let bytes = serde_json::to_vec(&pin_with_serialized_len(0x0010_0001)).unwrap();
            assert!(matches!(
                parse_route_value(&bytes),
                Err(StoreError::ValueTooLarge { bytes: reported, limit })
                    if reported == 0x0010_0001 && limit == 0x0010_0000
            ));
        }
        "parser_malformed_keeps_its_cause" => {
            let outcome = parse_route_value(b"{");
            if let Err(StoreError::InvalidValue { source }) = &outcome {
                assert_eq!(source.classify(), Category::Eof);
            } else {
                panic!("the malformed value must not decode");
            }
        }
        "parser_unknown_variant_is_rejected" => {
            let outcome = parse_route_value(br#"{"unknown":1}"#);
            if let Err(StoreError::InvalidValue { source }) = &outcome {
                assert_eq!(source.classify(), Category::Data);
            } else {
                panic!("the unknown variant must not decode");
            }
        }
        "counter_overflow_is_reported" => {
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
        "spend_overflow_is_reported" => {
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
        "increment_mismatch_keeps_value" => {
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
        "compare_and_set_mismatch_keeps_value" => {
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
        "capacity_evicts_earliest_expiry" => {
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
        "expired_entries_are_pruned_before_eviction" => {
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
        "overwrite_at_capacity_resets_expiry" => {
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
        _ => assert!(name.is_empty()),
    }
}
