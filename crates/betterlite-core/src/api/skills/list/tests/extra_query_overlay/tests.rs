#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::skills::fixtures::kwargs;
use crate::api::skills::list::call::ListSkillsCall;
use crate::api::skills::list::error::ListSkillsFailureError;
use crate::api::skills::list_fixtures::{
    Fixture, call_with, listing, logging, map, params_with_query, query,
};

fn call_with_query<'call>(
    limit: Option<i64>,
    page: Option<&'call str>,
    source: Option<&'call str>,
    extra: IndexMap<String, Json>,
    kwargs: &'call IndexMap<String, Json>,
) -> ListSkillsCall<'call> {
    call_with(params_with_query(extra), limit, page, source, kwargs)
}

async fn external_query(request: ListSkillsCall<'_>) -> Vec<(String, String)> {
    let fixture = Fixture::new();
    fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    requests[0].query_params.clone()
}

async fn failure_of(request: ListSkillsCall<'_>) -> ListSkillsFailureError {
    let fixture = Fixture::new();
    let failure = fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect_err("the wrong-typed overlay fails the call");
    assert!(fixture.http.requests.lock().expect("the fake").is_empty());
    failure
}

#[tokio::test]
async fn extra_query_overlay_replaces_the_assembled_limit() {
    let values = kwargs();
    let request = call_with_query(
        Some(5),
        None,
        None,
        map(&[("limit", Json::Number(Number::from(50_i64)))]),
        &values,
    );
    assert_eq!(external_query(request).await, query(&[("limit", "50")]));
}

#[tokio::test]
async fn extra_query_overlay_replaces_page_and_source() {
    let values = kwargs();
    let request = call_with_query(
        None,
        Some("1"),
        None,
        map(&[
            ("page", Json::Text("2".to_owned())),
            ("source", Json::Text("custom".to_owned())),
        ]),
        &values,
    );
    assert_eq!(
        external_query(request).await,
        query(&[("page", "2"), ("source", "custom")])
    );
}

#[tokio::test]
async fn extra_query_overlay_null_clears_an_assembled_value() {
    let values = kwargs();
    let request = call_with_query(Some(5), None, None, map(&[("limit", Json::Null)]), &values);
    assert!(external_query(request).await.is_empty());
}

#[tokio::test]
async fn extra_query_overlay_ignores_keys_the_provider_does_not_read() {
    let values = kwargs();
    let request = call_with_query(
        None,
        None,
        None,
        map(&[("cursor", Json::Text("next".to_owned()))]),
        &values,
    );
    assert!(external_query(request).await.is_empty());
}

#[tokio::test]
async fn extra_query_overlay_rejects_a_wrong_typed_limit() {
    let values = kwargs();
    let request = call_with_query(
        None,
        None,
        None,
        map(&[("limit", Json::Bool(true))]),
        &values,
    );
    let failure = failure_of(request).await;
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a wrong-typed field is a 400, not a provider error");
    };
    assert_eq!(details.body_param.as_deref(), Some("limit"));
    assert_eq!(
        details.message,
        "the limit field does not have the declared type"
    );
}

#[tokio::test]
async fn extra_query_overlay_rejects_a_wrong_typed_page() {
    let values = kwargs();
    let request = call_with_query(
        None,
        None,
        None,
        map(&[("page", Json::Number(Number::from(2_i64)))]),
        &values,
    );
    let failure = failure_of(request).await;
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a wrong-typed field is a 400, not a provider error");
    };
    assert_eq!(details.body_param.as_deref(), Some("page"));
    assert_eq!(
        details.message,
        "the page field does not have the declared type"
    );
}

#[tokio::test]
async fn extra_query_overlay_rejects_a_wrong_typed_source() {
    let values = kwargs();
    let request = call_with_query(
        None,
        None,
        None,
        map(&[("source", Json::Bool(false))]),
        &values,
    );
    let failure = failure_of(request).await;
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a wrong-typed field is a 400, not a provider error");
    };
    assert_eq!(details.body_param.as_deref(), Some("source"));
    assert_eq!(
        details.message,
        "the source field does not have the declared type"
    );
}

#[tokio::test]
async fn extra_query_overlay_rejects_a_fractional_limit() {
    let number: Number = serde_json::from_str("5.5").expect("5.5 is a JSON number");
    let values = kwargs();
    let request = call_with_query(
        None,
        None,
        None,
        map(&[("limit", Json::Number(number))]),
        &values,
    );
    let failure = failure_of(request).await;
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a fractional limit is a 400, not a provider error");
    };
    assert_eq!(details.body_param.as_deref(), Some("limit"));
    assert_eq!(
        details.message,
        "the limit field does not have the declared type"
    );
}
