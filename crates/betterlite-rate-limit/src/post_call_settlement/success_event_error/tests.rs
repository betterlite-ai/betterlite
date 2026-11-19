#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::limiter_records::ParallelSlotAcquisition;
use crate::post_call_settlement::tests::support::{
    LogCapture, RerankIo, SettlementFixture, SuccessFixture, install, stash_state,
};

#[tokio::test]
async fn logs_a_reconciliation_error_without_failing_the_response() {
    let capture = LogCapture::start();
    let fixture = SettlementFixture::default();
    let mut success = SuccessFixture::default();
    success
        .metadata
        .insert("internal_call_origin".to_owned(), Json::Bool(true));
    install(&success.slot, |state| {
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
    });
    fixture
        .settlement()
        .log_success_event(&success.callback(&RerankIo::default()))
        .await;
    assert!(capture.text().contains("Error in rate limit success event"));
    assert!(stash_state(&success.slot).parallel_slot.is_none());
    assert!(fixture.usage.pipelines().is_empty());
}

#[tokio::test]
async fn succeeds_without_a_stash() {
    let capture = LogCapture::start();
    let fixture = SettlementFixture::default();
    let success = SuccessFixture::default();
    fixture
        .settlement()
        .log_success_event(&success.callback(&RerankIo::default()))
        .await;
    assert!(!capture.text().contains("Error in rate limit success event"));
}
