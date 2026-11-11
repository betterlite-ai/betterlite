#![cfg(test)]

use std::time::Duration;

use http::HeaderMap;

use crate::api::skills::fixtures::{
    Fixture, SKILL_ID, call, call_with, hosted_params, kwargs, logging, skill_request,
};
use crate::api::skills::get_delete::call::SkillsCallParams;

#[tokio::test]
async fn timeout_default_uses_the_request_timeout_setting() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests[0].timeout, Duration::from_mins(100));
}

#[tokio::test]
async fn timeout_explicit_value_is_preserved() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: Some(Duration::from_secs(5)),
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests[0].timeout, Duration::from_secs(5));
}

#[tokio::test]
async fn timeout_default_zero_falls_back() {
    let mut fixture = Fixture::new();
    fixture.core.request_timeout_seconds = 30.0_f64;
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: Some(Duration::ZERO),
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests[0].timeout, Duration::from_secs(30));
}

#[tokio::test]
async fn timeout_is_not_consulted_for_hosted_calls() {
    let mut fixture = Fixture::new();
    fixture.core.request_timeout_seconds = -1.0_f64;
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get never resolves a timeout");
}
