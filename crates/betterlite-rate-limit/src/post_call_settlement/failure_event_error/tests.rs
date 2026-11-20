#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::limiter_records::{CacheCounterValue, ParallelSlotAcquisition, ReservationBackend};
use crate::post_call_settlement::failure_event::FailureEvent;
use crate::post_call_settlement::tests::support::{
    FakeCounterCache, LogCapture, SettlementFixture, WINDOW, install, scopes, stash_state,
    window_identity,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn logs_a_guarded_refund_error_without_failing_the_response() {
    let capture = LogCapture::start();
    let fixture = SettlementFixture::with_cache(FakeCounterCache::seeded(&[
        ("{project:p1}:window", CacheCounterValue::Int(100)),
        (
            "{project:p1}:tokens",
            CacheCounterValue::Text("not a number".to_owned()),
        ),
    ]));
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
        state.itpm_reserved_tokens = 100;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
        state.itpm_reserved_windows.insert(window_identity(
            "{project:p1}:tokens",
            ReservationBackend::Local,
        ));
    });
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert!(capture.text().contains("Error in rate limit failure event"));
    let state = stash_state(&slot);
    assert!(state.parallel_slot.is_none());
    assert!(!state.reservation_released);
}

#[tokio::test]
async fn succeeds_without_a_stash() {
    let capture = LogCapture::start();
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert!(!capture.text().contains("Error in rate limit failure event"));
}
