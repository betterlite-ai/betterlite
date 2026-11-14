#![cfg(test)]

use std::sync::Arc;

use crate::limiter_records::CounterRefund;
use crate::settlement_and_rollback::tests::support::{
    FakeCache, FakeScript, FixedClock, LogCapture, amounts, descriptor, key_list, reply,
    settlement, store, text,
};

const NOW: u64 = 1_700_000_000;

fn counter_refund(counter_key: &str, window_start: &str) -> CounterRefund {
    CounterRefund {
        window_key: "{model:gpt-4}:window".to_owned(),
        counter_key: counter_key.to_owned(),
        window_start: window_start.to_owned(),
        increment: 1,
    }
}

#[tokio::test]
async fn a_failed_refund_is_logged_and_the_rest_still_run() {
    let capture = LogCapture::start();
    let refunds = FakeScript::failing(vec![reply(&[1]), reply(&[1])], &[0]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, None, Some(refunds.clone()), false);
    settler
        .refund_counter_increments(&[
            counter_refund("{model:gpt-4}:requests", "1000"),
            counter_refund("{model:gpt-4}:tokens", "1000"),
        ])
        .await;
    let calls = refunds.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls.get(1).map(|call| call.keys.clone()),
        Some(key_list(&["{model:gpt-4}:window", "{model:gpt-4}:tokens"]))
    );
    assert_eq!(
        calls.get(1).map(|call| call.args.clone()),
        Some(vec![text("1000"), text("-1"), text("0")])
    );
    let logged = capture.text();
    assert!(logged.contains("WARN"));
    assert!(logged.contains("failed to refund a counter on rollback"));
    assert!(logged.contains("{model:gpt-4}:requests"));
}

#[tokio::test]
async fn a_failed_refund_leaves_the_over_limit_response_standing() {
    let capture = LogCapture::start();
    let script = FakeScript::new(vec![reply(&[0, 1, 100]), reply(&[1, 1, 9, 10])]);
    let refunds = FakeScript::failing(vec![reply(&[1])], &[0]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(script),
        Some(refunds.clone()),
        false,
    );
    let response = settler
        .atomic_check_and_increment_by_n(
            &[
                descriptor("model", "gpt-4", Some(10), None),
                descriptor("model", "gpt-3.5", Some(10), None),
            ],
            &[amounts(1, 0), amounts(1, 0)],
        )
        .await
        .expect("the over-limit decision stands without its refund");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(
        response
            .statuses
            .first()
            .and_then(|status| status.descriptor_value.clone()),
        Some("gpt-3.5".to_owned())
    );
    assert_eq!(refunds.calls().len(), 1);
    let logged = capture.text();
    assert!(logged.contains("WARN"));
    assert!(logged.contains("failed to refund a counter on rollback"));
}
