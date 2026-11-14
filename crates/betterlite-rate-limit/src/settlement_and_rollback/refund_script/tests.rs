#![cfg(test)]

use std::sync::Arc;

use crate::limiter_records::CounterRefund;
use crate::settlement_and_rollback::tests::support::{
    FakeCache, FakeScript, FixedClock, key_list, reply, settlement, store, text,
};

const NOW: u64 = 1_700_000_000;

fn counter_refund(
    window_key: &str,
    counter_key: &str,
    window_start: &str,
    increment: i64,
) -> CounterRefund {
    CounterRefund {
        window_key: window_key.to_owned(),
        counter_key: counter_key.to_owned(),
        window_start: window_start.to_owned(),
        increment,
    }
}

#[tokio::test]
async fn each_refund_calls_the_guarded_decrement_in_order() {
    let refunds = FakeScript::new(vec![reply(&[1]), reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, None, Some(refunds.clone()), false);
    settler
        .refund_counter_increments(&[
            counter_refund("{model:gpt-4}:window", "{model:gpt-4}:requests", "1000", 1),
            counter_refund("{model:gpt-4}:window", "{model:gpt-4}:tokens", "999", 3),
        ])
        .await;
    let calls = refunds.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-4}:window",
            "{model:gpt-4}:requests"
        ]))
    );
    assert_eq!(
        calls.first().map(|call| call.args.clone()),
        Some(vec![text("1000"), text("-1"), text("0")])
    );
    assert_eq!(
        calls.get(1).map(|call| call.keys.clone()),
        Some(key_list(&["{model:gpt-4}:window", "{model:gpt-4}:tokens"]))
    );
    assert_eq!(
        calls.get(1).map(|call| call.args.clone()),
        Some(vec![text("999"), text("-3"), text("0")])
    );
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn applied_groups_flatten_into_one_refund_round() {
    let refunds = FakeScript::new(vec![reply(&[1]), reply(&[1]), reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, None, Some(refunds.clone()), false);
    settler
        .refund_applied_descriptor_groups(&[
            vec![
                counter_refund("{model:gpt-4}:window", "{model:gpt-4}:requests", "100", 1),
                counter_refund("{model:gpt-4}:window", "{model:gpt-4}:tokens", "100", 2),
            ],
            vec![counter_refund(
                "{model:gpt-3.5}:window",
                "{model:gpt-3.5}:requests",
                "200",
                1,
            )],
        ])
        .await;
    let counter_keys: Vec<String> = refunds
        .calls()
        .iter()
        .map(|call| call.keys.get(1).cloned().unwrap_or_default())
        .collect();
    assert_eq!(
        counter_keys,
        vec![
            "{model:gpt-4}:requests".to_owned(),
            "{model:gpt-4}:tokens".to_owned(),
            "{model:gpt-3.5}:requests".to_owned(),
        ]
    );
}
