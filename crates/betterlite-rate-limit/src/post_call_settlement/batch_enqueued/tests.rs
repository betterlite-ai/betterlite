#![cfg(test)]

use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundError;
use betterlite_core::api::batch_enqueued_reserve::response::BatchResponseView;
use betterlite_openai::batches::records::BatchStatus;

use crate::post_call_settlement::batch_enqueued::BatchEnqueuedCall;
use crate::post_call_settlement::tests::support::{
    BatchCall, SettlementFixture, install, reservation, stash_state,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn ignores_a_response_that_is_not_a_batch() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.batch_enqueued_reservation = Some(reservation(500, &[("api_key", "k1")]));
    });
    let call = BatchEnqueuedCall {
        stash: &slot,
        response: None,
    };
    fixture
        .settlement()
        .handle_batch_enqueued_post_call(&call)
        .await
        .expect("the batch hook succeeds");
    assert!(fixture.store.calls().is_empty());
    assert!(stash_state(&slot).batch_enqueued_reservation.is_some());
}

#[tokio::test]
async fn saves_without_popping_on_a_non_terminal_status() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.batch_enqueued_reservation = Some(reservation(500, &[("api_key", "k1")]));
    });
    let view = BatchResponseView {
        id: "batch_in_progress".to_owned(),
        status: BatchStatus::InProgress,
    };
    let call = BatchEnqueuedCall {
        stash: &slot,
        response: Some(&view),
    };
    fixture
        .settlement()
        .handle_batch_enqueued_post_call(&call)
        .await
        .expect("the batch hook succeeds");
    assert_eq!(
        fixture.store.calls(),
        vec![BatchCall {
            kind: "save",
            batch_id: Some("batch_in_progress".to_owned()),
            tokens: Some(500),
        }]
    );
    assert!(stash_state(&slot).batch_enqueued_reservation.is_none());
}

#[tokio::test]
async fn propagates_a_failed_save_and_keeps_the_reservation() {
    let fixture = SettlementFixture::default();
    fixture.store.fail_only("save");
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.batch_enqueued_reservation = Some(reservation(500, &[("api_key", "k1")]));
    });
    let view = BatchResponseView {
        id: "batch-1".to_owned(),
        status: BatchStatus::Validating,
    };
    let call = BatchEnqueuedCall {
        stash: &slot,
        response: Some(&view),
    };
    let result = fixture
        .settlement()
        .handle_batch_enqueued_post_call(&call)
        .await;
    assert!(matches!(
        result,
        Err(BatchEnqueuedRefundError::Encode { .. })
    ));
    assert!(stash_state(&slot).batch_enqueued_reservation.is_some());
}
