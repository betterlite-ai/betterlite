#![cfg(test)]

use crate::call_context::{BillingPolicy, CallContext, CallInputs, Phase};
use rstest::rstest;
use std::sync::Barrier;
use std::thread;
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

#[rstest]
#[case::concurrent_calls(11, 22)]
#[case::different_clock_values(31, 47)]
fn call_context_tests_behavior(#[case] first: u64, #[case] second: u64) {
    let barrier = Barrier::new(2);
    thread::scope(|scope| {
        let run = |number| {
            let context = CallContext::start(CallInputs {
                id: || Uuid::from_u128(u128::from(number)),
                now: || UNIX_EPOCH.checked_add(Duration::from_secs(number)).unwrap(),
                deadline: None,
                privacy: false,
            });
            barrier.wait();
            let child = context.post_response().child(Uuid::from_u128(99));
            assert_eq!(child.view().state.phase, Phase::PostResponse);
            assert_eq!(context.view().state.phase, Phase::Request);
            assert_eq!(context.view().state.billing, BillingPolicy::Include);
            *context.view().state
        };
        let left = scope.spawn(move || run(first));
        let right = scope.spawn(move || run(second));
        let left_view = left.join().unwrap();
        let right_view = right.join().unwrap();
        assert_ne!(left_view.id, right_view.id);
        assert_eq!(
            left_view.billing_time,
            UNIX_EPOCH.checked_add(Duration::from_secs(first)).unwrap()
        );
        assert_eq!(
            right_view.billing_time,
            UNIX_EPOCH.checked_add(Duration::from_secs(second)).unwrap()
        );
    });
}
