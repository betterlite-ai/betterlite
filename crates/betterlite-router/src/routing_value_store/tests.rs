#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use rust_decimal::Decimal;

use super::codec::parse_route_value;
use super::{
    CompareOutcome, PinValue, RouteIncrement, RouteKey, RouteValue, RoutingValueStore,
    SchedulerTuple, StoreRead, StoreSettings,
};

mod boundaries;

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
#[case::concurrent_set_add_keeps_every_member("concurrent_set_add_keeps_every_member")]
#[case::increment_counter_from_missing("increment_counter_from_missing")]
#[case::increment_counter_existing("increment_counter_existing")]
#[case::increment_spend_from_missing("increment_spend_from_missing")]
#[case::increment_spend_existing("increment_spend_existing")]
#[case::increment_counter_by_spend("increment_counter_by_spend")]
#[case::increment_spend_by_counter("increment_spend_by_counter")]
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
        "counter_round_trip" => {
            let store = store_with(None, &clock());
            let key = route_key("global_router:a:tpm:00-00");
            store.set(&key, &counter(7), None).unwrap();
            assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(7)));
        }
        "spend_round_trip" => {
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
        "pin_round_trip" => {
            let store = store_with(None, &clock());
            let key = route_key("affinity:user");
            let value = RouteValue::Pin(PinValue {
                model_id: "deployment-1".to_owned(),
            });
            store.set(&key, &value, None).unwrap();
            assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
        }
        "scheduler_tuple_round_trip" => {
            let store = store_with(None, &clock());
            let key = route_key("scheduler:queue:model");
            let value = RouteValue::SchedulerTuple(SchedulerTuple {
                priority: 3,
                request_id: "req-1".to_owned(),
            });
            store.set(&key, &value, None).unwrap();
            assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
        }
        "members_round_trip" => {
            let store = store_with(None, &clock());
            let key = route_key("dynamic_rate:00-00:model");
            let value = RouteValue::Members(["a".to_owned(), "b".to_owned()].into_iter().collect());
            store.set(&key, &value, None).unwrap();
            assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
        }
        "pipeline_writes_every_entry" => {
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
        "set_add_accumulates_members" => {
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
        "concurrent_set_add_keeps_every_member" => {
            let store = Arc::new(store_with(None, &clock()));
            let key = route_key("dynamic_rate:00-00:model");
            std::thread::scope(|scope| {
                for index in 0..16_u32 {
                    let shared = Arc::clone(&store);
                    let route_key = key.clone();
                    scope.spawn(move || {
                        shared
                            .set_add(&route_key, &[format!("team-{index}")], None)
                            .unwrap();
                    });
                }
            });
            let expected = (0..16_u32).map(|index| format!("team-{index}")).collect();
            assert_eq!(
                store.get(&key).unwrap(),
                StoreRead::Value(RouteValue::Members(expected))
            );
        }
        "increment_counter_from_missing" => {
            let store = store_with(None, &clock());
            let key = route_key("in-flight");
            let value = store
                .increment(&key, &RouteIncrement::Counter(3), None)
                .unwrap();
            assert_eq!(value, counter(3));
            assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(3)));
        }
        "increment_counter_existing" => {
            let store = store_with(None, &clock());
            let key = route_key("in-flight");
            store.set(&key, &counter(2), None).unwrap();
            let value = store
                .increment(&key, &RouteIncrement::Counter(5), None)
                .unwrap();
            assert_eq!(value, counter(7));
        }
        "increment_spend_from_missing" => {
            let store = store_with(None, &clock());
            let key = route_key("spend");
            let value = store
                .increment(&key, &RouteIncrement::Spend(Decimal::new(25, 2)), None)
                .unwrap();
            assert_eq!(value, RouteValue::Spend(Decimal::new(25, 2)));
        }
        "increment_spend_existing" => {
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
        "increment_counter_by_spend" => {
            let store = store_with(None, &clock());
            let key = route_key("mixed-counter");
            store.set(&key, &counter(2), None).unwrap();
            let value = store
                .increment(&key, &RouteIncrement::Spend(Decimal::new(5, 1)), None)
                .unwrap();
            assert_eq!(value, RouteValue::Spend(Decimal::new(25, 1)));
        }
        "increment_spend_by_counter" => {
            let store = store_with(None, &clock());
            let key = route_key("mixed-spend");
            store
                .set(&key, &RouteValue::Spend(Decimal::new(25, 2)), None)
                .unwrap();
            let value = store
                .increment(&key, &RouteIncrement::Counter(2), None)
                .unwrap();
            assert_eq!(value, RouteValue::Spend(Decimal::new(225, 2)));
        }
        "increment_keeps_live_expiry" => {
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
        "compare_and_set_first_writer" => {
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
        "compare_and_set_keeps_winner" => {
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
        "compare_and_set_swaps_matching" => {
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
        "batch_get_keeps_order_and_duplicates" => {
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
        "expiry_equality_is_readable" => {
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
        "expired_read_is_missing" => {
            let clock = clock();
            let store = store_with(None, &clock);
            let key = route_key("counter");
            at(&clock, 0.0_f64);
            store.set(&key, &counter(1), Some(10.0_f64)).unwrap();
            at(&clock, 11.0_f64);
            assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
            assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
        }
        "zero_ttl_is_readable_then_expires" => {
            let clock = clock();
            let store = store_with(None, &clock);
            let key = route_key("counter");
            at(&clock, 0.0_f64);
            store.set(&key, &counter(1), Some(0.0_f64)).unwrap();
            assert_eq!(store.get(&key).unwrap(), StoreRead::Value(counter(1)));
            at(&clock, 0.5_f64);
            assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
        }
        "default_ttl_is_six_hundred_seconds" => {
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
        "oversized_value_is_not_cached" => {
            let store = store_with(None, &clock());
            let key = route_key("huge");
            // 0x0010_0001 is one byte past the 1 MiB item cap
            let value = pin_with_serialized_len(0x0010_0001);
            store.set(&key, &value, None).unwrap();
            assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
        }
        "capacity_zero_stores_nothing" => {
            let store = store_with(Some(0), &clock());
            let key = route_key("counter");
            store.set(&key, &counter(1), None).unwrap();
            assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
        }
        "delete_removes_value" => {
            let store = store_with(None, &clock());
            let key = route_key("counter");
            store.set(&key, &counter(1), None).unwrap();
            store.delete(&key);
            assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
        }
        _ => assert!(name.is_empty()),
    }
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
