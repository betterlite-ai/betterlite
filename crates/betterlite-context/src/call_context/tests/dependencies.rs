#![cfg(test)]

use crate::call_context::{CallContext, CallInputs};
use rstest::rstest;
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

#[rstest]
#[case::clock_read_once(1)]
#[case::billing_stays_pinned(2)]
fn call_context_tests_behavior(#[case] seconds: u64) {
    let mut reads = 0_u32;
    let context = CallContext::start(CallInputs {
        id: Uuid::new_v4,
        now: || {
            reads = reads.checked_add(1).unwrap();
            UNIX_EPOCH
                .checked_add(Duration::from_secs(seconds))
                .unwrap()
        },
        deadline: None,
        privacy: false,
    });
    let post_response = context.post_response();
    let child = post_response.child(Uuid::from_u128(7));
    assert_eq!(reads, 1);
    assert_eq!(context.view().state.id.get_version_num(), 4);
    assert_eq!(
        child.view().state.billing_time,
        UNIX_EPOCH
            .checked_add(Duration::from_secs(seconds))
            .unwrap()
    );
    assert_eq!(post_response.view().state.id, context.view().state.id);
}
