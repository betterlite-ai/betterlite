#![cfg(test)]

use crate::limiter_records::ParallelSlotAcquisition;
use crate::post_call_settlement::success_hook::SuccessHook;
use crate::post_call_settlement::tests::support::{SettlementFixture, install, stash_state};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn releases_the_stashed_slot_in_the_post_call_hook() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
    });
    let hook = SuccessHook {
        stash: &slot,
        call_id: None,
        response: None,
    };
    fixture
        .settlement()
        .post_call_success_hook(&hook, None)
        .await;
    assert!(stash_state(&slot).parallel_slot.is_none());
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
    let other_call = SuccessHook {
        stash: &slot,
        call_id: Some("call-b"),
        response: None,
    };
    fixture
        .settlement()
        .post_call_success_hook(&other_call, None)
        .await;
    assert!(stash_state(&slot).parallel_slot.is_some());
    let owning_call = SuccessHook {
        stash: &slot,
        call_id: Some("call-a"),
        response: None,
    };
    fixture
        .settlement()
        .post_call_success_hook(&owning_call, None)
        .await;
    assert!(stash_state(&slot).parallel_slot.is_none());
}
