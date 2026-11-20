#![cfg(test)]

use betterlite_core::api::batch_enqueued_reserve::response::BatchResponseView;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_openai::batches::records::BatchStatus;

use crate::limiter_records::{ParallelSlotAcquisition, RateLimitType};
use crate::post_call_settlement::success_hook::SuccessHook;
use crate::post_call_settlement::tests::support::{
    LogCapture, SettlementFixture, install, number, rate_limit_response, reservation, stash_state,
    status,
};
use crate::request_stash::RequestStashSlot;

fn install_failing_batch_slot(slot: &RequestStashSlot) {
    install(slot, |state| {
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
        state.rate_limit_response = Some(rate_limit_response(vec![status(
            "user",
            RateLimitType::Tokens,
            1000,
            750,
        )]));
        state.batch_enqueued_reservation = Some(reservation(500, &[("api_key", "k1")]));
    });
}

#[tokio::test]
async fn logs_a_batch_error_and_still_publishes_headers() {
    let capture = LogCapture::start();
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install_failing_batch_slot(&slot);
    fixture.store.fail_only("pop");
    let view = BatchResponseView {
        id: "batch-1".to_owned(),
        status: BatchStatus::Completed,
    };
    let hook = SuccessHook {
        stash: &slot,
        call_id: None,
        response: Some(&view),
    };
    let mut target = AdditionalHeaders::new();
    fixture
        .settlement()
        .post_call_success_hook(&hook, Some(&mut target))
        .await;
    assert!(
        capture
            .text()
            .contains("Error in batch enqueued-token post-call hook")
    );
    assert_eq!(
        target.get("x-ratelimit-user-remaining-tokens"),
        Some(&number(750))
    );
    assert!(stash_state(&slot).parallel_slot.is_none());
}

#[tokio::test]
async fn succeeds_without_a_batch_response() {
    let capture = LogCapture::start();
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.rate_limit_response = Some(rate_limit_response(vec![status(
            "user",
            RateLimitType::Tokens,
            1000,
            750,
        )]));
    });
    let hook = SuccessHook {
        stash: &slot,
        call_id: None,
        response: None,
    };
    let mut target = AdditionalHeaders::new();
    fixture
        .settlement()
        .post_call_success_hook(&hook, Some(&mut target))
        .await;
    assert!(
        !capture
            .text()
            .contains("Error in batch enqueued-token post-call hook")
    );
    assert_eq!(
        target.get("x-ratelimit-user-limit-tokens"),
        Some(&number(1000))
    );
}
