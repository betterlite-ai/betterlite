#![cfg(test)]

use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::api::skills::fixtures::{CALL_ID, kwargs, text};
use crate::api::skills::list_fixtures::{
    Fixture, HostedCall, call_with, hosted_params, listing, logging, map,
};

async fn hosted_call(limit: Option<i64>) -> HostedCall {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), limit, None, None, &values);
    fixture
        .service()
        .list_skills(listing(&hosted, &mut logging()))
        .await
        .expect("the hosted list succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    calls[0].clone()
}

#[tokio::test]
async fn hosted_pagination_defaults_limit_to_20_and_offset_to_zero() {
    let recorded = hosted_call(None).await;
    assert_eq!((recorded.limit, recorded.offset), (20, 0));
}

#[tokio::test]
async fn hosted_pagination_forwards_an_explicit_limit() {
    let recorded = hosted_call(Some(50)).await;
    assert_eq!((recorded.limit, recorded.offset), (50, 0));
}

#[tokio::test]
async fn hosted_pagination_keeps_a_negative_limit() {
    let recorded = hosted_call(Some(-5)).await;
    assert_eq!(recorded.limit, -5);
}

#[tokio::test]
async fn hosted_pagination_treats_a_zero_limit_as_absent() {
    let recorded = hosted_call(Some(0)).await;
    assert_eq!((recorded.limit, recorded.offset), (20, 0));
}

#[tokio::test]
async fn hosted_pagination_omits_page_source_and_extra_query() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut params = hosted_params();
    params.extra_query = Some(map(&[
        ("limit", Json::Number(Number::from(7_i64))),
        ("page", text("2")),
    ]));
    let hosted = call_with(params, Some(20), Some("tok"), Some("custom"), &values);
    fixture
        .service()
        .list_skills(listing(&hosted, &mut logging()))
        .await
        .expect("the hosted list succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(
        calls[0],
        HostedCall {
            limit: 20,
            offset: 0,
            principal: None,
            litellm_call_id: Some(CALL_ID.to_owned()),
        }
    );
    assert!(fixture.http.requests.lock().expect("the fake").is_empty());
}
