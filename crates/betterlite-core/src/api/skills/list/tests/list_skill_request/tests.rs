#![cfg(test)]

use betterlite_anthropic::credentials::ANTHROPIC_VERSION;
use http::header::HeaderMap;
use secrecy::SecretString;

use crate::api::skills::fixtures::kwargs;
use crate::api::skills::get_delete::call::SkillsCallParams;
use crate::api::skills::list_fixtures::{
    Fixture, call, call_with, extra_headers, header_values, listing, logging, params_with_base,
    unauthenticated_settings,
};

#[tokio::test]
async fn list_skill_request_carries_the_validated_headers() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "x-api-key"),
        ["sk-ant-test"]
    );
    assert!(
        requests[0]
            .headers
            .get("x-api-key")
            .expect("the auth header exists")
            .is_sensitive()
    );
    assert_eq!(
        header_values(&requests[0].headers, "anthropic-version"),
        [ANTHROPIC_VERSION]
    );
    assert_eq!(
        header_values(&requests[0].headers, "anthropic-beta"),
        ["skills-2025-10-02"]
    );
    assert_eq!(
        header_values(&requests[0].headers, "content-type"),
        ["application/json"]
    );
}

#[tokio::test]
async fn list_skill_request_keeps_caller_extra_headers() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: extra_headers("anthropic-beta", "custom-beta"),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, None, None, None, &values);
    fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "anthropic-beta"),
        ["custom-beta", "skills-2025-10-02"]
    );
}

#[tokio::test]
async fn list_skill_request_targets_the_skills_endpoint() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    assert_eq!(requests[0].url, "https://api.anthropic.com/v1/skills");
}

#[tokio::test]
async fn list_skill_request_prefers_the_call_api_base() {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(
        params_with_base(Some("https://gateway.example")),
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
    assert_eq!(requests[0].url, "https://gateway.example/v1/skills");
}

#[tokio::test]
async fn list_skill_request_ignores_an_empty_api_base() {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(params_with_base(Some("")), None, None, None, &values);
    fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    assert_eq!(requests[0].url, "https://api.anthropic.com/v1/skills");
}

#[tokio::test]
async fn list_skill_transformation_prefers_the_call_api_key() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: Some(SecretString::from("sk-ant-call")),
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, None, None, None, &values);
    fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect("the external list succeeds");
    let requests = fixture.http.requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "x-api-key"),
        ["sk-ant-call"]
    );
}

#[tokio::test]
async fn list_skill_request_reports_a_missing_credential() {
    let mut fixture = Fixture::new();
    fixture.anthropic = unauthenticated_settings();
    let values = kwargs();
    let failure = fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect_err("a missing credential fails");
    assert_eq!(
        failure.error.to_string(),
        "ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN is required for Skills API"
    );
    assert!(fixture.http.requests.lock().expect("the fake").is_empty());
}
