#![cfg(test)]

use crate::limiter_records::ParallelSlotAcquisition;
use crate::post_call_settlement::success_event::SuccessCallback;
use crate::post_call_settlement::tests::support::{
    RerankIo, SettlementFixture, SuccessFixture, install, stash_state,
};

#[tokio::test]
async fn releases_the_stashed_slot_on_success() {
    let fixture = SettlementFixture::default();
    let success = SuccessFixture::default();
    install(&success.slot, |state| {
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
    });
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo::default().reconciliation(),
        })
        .await;
    assert!(stash_state(&success.slot).parallel_slot.is_none());
}

#[tokio::test]
async fn releases_only_for_the_owning_call() {
    let fixture = SettlementFixture::default();
    let mut success = SuccessFixture {
        call_id: Some("call-b".to_owned()),
        ..SuccessFixture::default()
    };
    install(&success.slot, |state| {
        state.owner_litellm_call_id = Some("call-a".to_owned());
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
    });
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo::default().reconciliation(),
        })
        .await;
    assert!(stash_state(&success.slot).parallel_slot.is_some());
    success.call_id = Some("call-a".to_owned());
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo::default().reconciliation(),
        })
        .await;
    assert!(stash_state(&success.slot).parallel_slot.is_none());
}
