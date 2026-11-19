#![cfg(test)]

use betterlite_core::api::batch_enqueued_reserve::response::BatchResponseView;
use betterlite_openai::batches::records::BatchStatus;

use crate::post_call_settlement::batch_enqueued::BatchEnqueuedCall;
use crate::post_call_settlement::tests::support::{
    BatchCall, SettlementFixture, install, reservation, stash_state,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn saves_the_stashed_reservation_under_the_response_batch_id() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.batch_enqueued_reservation = Some(reservation(700, &[("api_key", "k1")]));
    });
    let view = BatchResponseView {
        id: "batch-1".to_owned(),
        status: BatchStatus::Validating,
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
            batch_id: Some("batch-1".to_owned()),
            tokens: Some(700),
        }]
    );
    assert!(stash_state(&slot).batch_enqueued_reservation.is_none());
}

#[tokio::test]
async fn saves_nothing_without_a_stashed_reservation() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |_state| {});
    let view = BatchResponseView {
        id: "batch-1".to_owned(),
        status: BatchStatus::Validating,
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
    assert!(fixture.store.calls().is_empty());
}
