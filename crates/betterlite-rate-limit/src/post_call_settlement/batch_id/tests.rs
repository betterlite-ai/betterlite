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
#[case::encoded_batch_marker("bGl0ZWxsbV9wcm94eTptb2RlbF9pZCxtO2xsbV9iYXRjaF9pZDpiLTE", "b-1")]
#[case::encoded_generic_response_marker(
    "bGl0ZWxsbV9wcm94eTo7Z2VuZXJpY19yZXNwb25zZV9pZDpyZXNwLTk",
    "resp-9"
)]
#[case::encoded_without_a_marker("bGl0ZWxsbV9wcm94eTpwbGFpbg", "litellm_proxy:plain")]
#[case::plain_provider_id("batch_abc", "batch_abc")]
#[tokio::test]
async fn saves_under_the_canonical_batch_id(#[case] response_id: &str, #[case] expected_id: &str) {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.batch_enqueued_reservation = Some(reservation(500, &[("api_key", "k1")]));
    });
    let view = BatchResponseView {
        id: response_id.to_owned(),
        status: BatchStatus::Validating,
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
        fixture.store.calls(),
        vec![BatchCall {
            kind: "save",
            batch_id: Some(expected_id.to_owned()),
            tokens: Some(500),
        }]
    );
}

#[tokio::test]
async fn pops_under_the_same_canonical_batch_id() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.batch_enqueued_reservation = Some(reservation(500, &[("api_key", "k1")]));
    });
    let view = BatchResponseView {
        id: "bGl0ZWxsbV9wcm94eTptb2RlbF9pZCxtO2xsbV9iYXRjaF9pZDpiLTE".to_owned(),
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
    assert_eq!(fixture.store.calls()[0].batch_id.as_deref(), Some("b-1"));
    assert_eq!(fixture.store.calls()[1].batch_id.as_deref(), Some("b-1"));
}
