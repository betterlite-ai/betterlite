#![cfg(test)]

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use http::HeaderMap;
use secrecy::SecretString;

use crate::api::skills::fixtures::{
    API_BASE, Fixture, SKILL_ID, call, call_with, extra_headers, header_values, kwargs, logging,
    skill_request,
};
use crate::api::skills::get_delete::call::SkillsCallParams;

#[tokio::test]
async fn get_skill_request_carries_the_validated_headers() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
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
async fn get_skill_request_keeps_caller_extra_headers() {
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
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "anthropic-beta"),
        ["custom-beta", "skills-2025-10-02"]
    );
}

#[tokio::test]
async fn get_skill_request_reports_a_missing_credential() {
    let mut fixture = Fixture::new();
    fixture.anthropic = AnthropicSettings {
        api_key: None,
        auth_token: None,
        api_base: API_BASE.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    };
    let values = kwargs();
    let failure = fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect_err("a missing credential fails");
    assert_eq!(
        failure.error.to_string(),
        "ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN is required for Skills API"
    );
    assert!(
        fixture
            .http
            .get_requests
            .lock()
            .expect("the fake")
            .is_empty()
    );
}

#[tokio::test]
async fn delete_skill_request_carries_the_validated_headers() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external delete succeeds");
    let requests = fixture.http.delete_requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "x-api-key"),
        ["sk-ant-test"]
    );
    assert_eq!(
        header_values(&requests[0].headers, "anthropic-beta"),
        ["skills-2025-10-02"]
    );
}

#[tokio::test]
async fn transformation_prefers_the_call_api_key() {
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
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "x-api-key"),
        ["sk-ant-call"]
    );
}
