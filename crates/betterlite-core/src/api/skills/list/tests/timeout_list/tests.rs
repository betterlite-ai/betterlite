#![cfg(test)]

use std::time::Duration;

use crate::api::skills::fixtures::kwargs;
use crate::api::skills::list_fixtures::{
    Fixture, call, call_with, hosted_params, listing, logging, params_with_timeout,
};

#[tokio::test]
async fn timeout_list_uses_the_request_timeout_setting() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    assert_eq!(requests[0].timeout, Duration::from_mins(100));
}

#[tokio::test]
async fn timeout_list_explicit_value_is_preserved() {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(
        params_with_timeout(Some(Duration::from_secs(5))),
        None,
        None,
        None,
        &values,
    );
    fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    assert_eq!(requests[0].timeout, Duration::from_secs(5));
}

#[tokio::test]
async fn timeout_list_zero_falls_back() {
    let mut fixture = Fixture::new();
    fixture.core.request_timeout_seconds = 30.0_f64;
    let values = kwargs();
    let request = call_with(
        params_with_timeout(Some(Duration::ZERO)),
        None,
        None,
        None,
        &values,
    );
    fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    assert_eq!(requests[0].timeout, Duration::from_secs(30));
}

#[tokio::test]
async fn timeout_list_ignores_the_setting_for_hosted_calls() {
    let mut fixture = Fixture::new();
    fixture.core.request_timeout_seconds = -1.0_f64;
    let values = kwargs();
    let hosted = call_with(hosted_params(), None, None, None, &values);
    fixture
        .service()
        .list_skills(listing(&hosted, &mut logging()))
        .await
        .expect("the hosted list never resolves a timeout");
}
