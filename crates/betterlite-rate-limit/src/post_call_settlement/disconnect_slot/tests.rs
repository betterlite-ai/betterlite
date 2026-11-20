#![cfg(test)]

use crate::limiter_records::ParallelSlotAcquisition;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, install, scopes, stash_state,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn releases_the_stashed_slot_on_disconnect() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
    });
    fixture.settlement().release_slot_on_disconnect(&slot).await;
    assert!(stash_state(&slot).parallel_slot.is_none());
}

#[tokio::test]
async fn releases_the_slot_of_a_call_the_key_no_longer_limits() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.owner_litellm_call_id = Some("call-a".to_owned());
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    fixture.settlement().release_slot_on_disconnect(&slot).await;
    let state = stash_state(&slot);
    assert!(state.parallel_slot.is_none());
    assert_eq!(state.reserved_tokens, 1000);
    assert!(!state.reservation_released);
}

#[tokio::test]
async fn does_nothing_without_a_stash_or_slot() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    fixture.settlement().release_slot_on_disconnect(&slot).await;
    assert!(slot.get().is_none());
}
