#![cfg(test)]

use crate::api::skills::fixtures::kwargs;
use crate::api::skills::list_fixtures::{
    Fixture, call, call_with, listing, logging, params, query,
};

async fn external_query(
    limit: Option<i64>,
    page: Option<&str>,
    source: Option<&str>,
) -> Vec<(String, String)> {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(params(), limit, page, source, &values);
    fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    requests[0].query_params.clone()
}

#[tokio::test]
async fn list_skill_parameter_assembly_passes_limit_page_and_source() {
    let assembled = external_query(Some(5), Some("tok"), Some("custom")).await;
    assert_eq!(
        assembled,
        query(&[("limit", "5"), ("page", "tok"), ("source", "custom")])
    );
}

#[tokio::test]
async fn list_skill_params_omit_absent_values() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    assert!(requests[0].query_params.is_empty());
    drop(requests);
}

#[tokio::test]
async fn list_skill_params_drop_a_zero_limit_from_the_query() {
    assert!(external_query(Some(0), None, None).await.is_empty());
}

#[tokio::test]
async fn list_skill_params_drop_an_empty_page_and_source() {
    assert!(external_query(None, Some(""), Some("")).await.is_empty());
}

#[tokio::test]
async fn list_skill_params_keep_a_negative_limit() {
    assert_eq!(
        external_query(Some(-1), None, None).await,
        query(&[("limit", "-1")])
    );
}
