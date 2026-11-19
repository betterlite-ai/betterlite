#![cfg(test)]

use betterlite_core::api::batch_enqueued_reserve::response::BatchResponseView;
use betterlite_openai::batches::records::BatchStatus;
use rstest::rstest;

use crate::post_call_settlement::batch_enqueued::BatchEnqueuedCall;
use crate::post_call_settlement::tests::support::{
    BatchCall, SettlementFixture, install, reservation,
};
use crate::request_stash::RequestStashSlot;

#[rstest]
#[case::completed(BatchStatus::Completed, vec!["save", "pop", "refund"])]
#[case::failed(BatchStatus::Failed, vec!["save", "pop", "refund"])]
#[case::expired(BatchStatus::Expired, vec!["save", "pop", "refund"])]
#[case::cancelling(BatchStatus::Cancelling, vec!["save", "pop", "refund"])]
#[case::cancelled(BatchStatus::Cancelled, vec!["save", "pop", "refund"])]
#[case::validating(BatchStatus::Validating, vec!["save"])]
#[case::in_progress(BatchStatus::InProgress, vec!["save"])]
#[case::finalizing(BatchStatus::Finalizing, vec!["save"])]
#[tokio::test]
async fn refunds_a_popped_reservation_only_for_terminal_statuses(
    #[case] status: BatchStatus,
    #[case] expected: Vec<&'static str>,
) {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    let pending = reservation(700, &[("api_key", "k1")]);
    install(&slot, |state| {
        state.batch_enqueued_reservation = Some(pending.clone());
    });
    fixture.store.pop_returns(&pending);
    let view = BatchResponseView {
        id: "batch-1".to_owned(),
        status,
    };
    let call = BatchEnqueuedCall {
        stash: &slot,
        response: Some(&view),
    };
    fixture
        .settlement()
        .handle_batch_post_call(&call)
        .await
        .expect("the batch hook succeeds");
    let kinds: Vec<&str> = fixture.store.calls().iter().map(|call| call.kind).collect();
    assert_eq!(kinds, expected);
}

#[tokio::test]
async fn refunds_the_reservation_the_store_returns() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.batch_enqueued_reservation = Some(reservation(700, &[("api_key", "k1")]));
    });
    fixture
        .store
        .pop_returns(&reservation(900, &[("api_key", "k1")]));
    let view = BatchResponseView {
        id: "batch-1".to_owned(),
        status: BatchStatus::Completed,
    };
    let call = BatchEnqueuedCall {
        stash: &slot,
        response: Some(&view),
    };
    fixture
        .settlement()
        .handle_batch_post_call(&call)
        .await
        .expect("the batch hook succeeds");
    assert_eq!(
        fixture.store.calls()[2],
        BatchCall {
            kind: "refund",
            batch_id: None,
            tokens: Some(900),
        }
    );
}

#[tokio::test]
async fn refunds_nothing_when_the_store_has_already_popped_the_record() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.batch_enqueued_reservation = Some(reservation(700, &[("api_key", "k1")]));
    });
    let view = BatchResponseView {
        id: "batch-1".to_owned(),
        status: BatchStatus::Completed,
    };
    let call = BatchEnqueuedCall {
        stash: &slot,
        response: Some(&view),
    };
    fixture
        .settlement()
        .handle_batch_post_call(&call)
        .await
        .expect("the batch hook succeeds");
    let kinds: Vec<&str> = fixture.store.calls().iter().map(|call| call.kind).collect();
    assert_eq!(kinds, vec!["save", "pop"]);
}
