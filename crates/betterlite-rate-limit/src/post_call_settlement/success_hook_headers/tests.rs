#![cfg(test)]

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;

use crate::limiter_records::{ParallelSlotAcquisition, RateLimitType};
use crate::post_call_settlement::success_hook::SuccessHook;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, headers, install, number, rate_limit_response, stash_state, status,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn merges_the_rate_limit_headers_into_the_response() {
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
    let mut target = headers(&[("x-request-id", Json::Text("abc".to_owned()))]);
    fixture
        .settlement()
        .post_call_success_hook(&hook, Some(&mut target))
        .await;
    assert_eq!(
        target.get("x-ratelimit-user-remaining-tokens"),
        Some(&number(750))
    );
    assert_eq!(
        target.get("x-ratelimit-user-limit-tokens"),
        Some(&number(1000))
    );
    assert_eq!(
        target.get("x-request-id"),
        Some(&Json::Text("abc".to_owned()))
    );
}

#[tokio::test]
async fn merges_headers_from_a_stash_owned_by_another_call() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.owner_litellm_call_id = Some("call-a".to_owned());
        state.parallel_slot = Some(ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: Vec::new(),
        });
        state.rate_limit_response = Some(rate_limit_response(vec![status(
            "user",
            RateLimitType::Requests,
            10,
            9,
        )]));
    });
    let hook = SuccessHook {
        stash: &slot,
        call_id: Some("call-b"),
        response: None,
    };
    let mut target = AdditionalHeaders::new();
    fixture
        .settlement()
        .post_call_success_hook(&hook, Some(&mut target))
        .await;
    assert_eq!(
        target.get("x-ratelimit-user-remaining-requests"),
        Some(&number(9))
    );
    assert_eq!(
        target.get("x-ratelimit-user-limit-requests"),
        Some(&number(10))
    );
    assert!(stash_state(&slot).parallel_slot.is_some());
}

#[tokio::test]
async fn leaves_the_response_alone_without_statuses() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |_state| {});
    let hook = SuccessHook {
        stash: &slot,
        call_id: None,
        response: None,
    };
    let mut target = headers(&[("x-request-id", Json::Text("abc".to_owned()))]);
    fixture
        .settlement()
        .post_call_success_hook(&hook, Some(&mut target))
        .await;
    assert_eq!(target.len(), 1);
}
