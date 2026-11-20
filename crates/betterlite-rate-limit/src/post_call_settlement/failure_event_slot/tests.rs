#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::limiter_records::ParallelSlotAcquisition;
use crate::post_call_settlement::failure_event::FailureEvent;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, stash_state,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn releases_the_stashed_slot_on_failure() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
    });
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    let state = stash_state(&slot);
    assert!(state.parallel_slot.is_none());
    assert!(!state.reservation_released);
    assert!(fixture.usage.pipelines().is_empty());
}

#[tokio::test]
async fn releases_only_for_the_owning_call() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.owner_litellm_call_id = Some("call-a".to_owned());
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
    });
    let other_call = FailureEvent {
        stash: &slot,
        call_id: Some("call-b"),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&other_call).await;
    assert!(stash_state(&slot).parallel_slot.is_some());
    let owning_call = FailureEvent {
        stash: &slot,
        call_id: Some("call-a"),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&owning_call).await;
    assert!(stash_state(&slot).parallel_slot.is_none());
}
