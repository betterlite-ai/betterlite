#![cfg(test)]

use betterlite_error::error::ErrorClass;
use http::header::HeaderMap;
use secrecy::SecretString;

use crate::api::skills::fixtures::kwargs;
use crate::api::skills::get_delete::call::SkillsCallParams;
use crate::api::skills::list_fixtures::{
    Fixture, call, call_with, listing, logging, unauthenticated_settings,
};

#[tokio::test]
async fn validation_failure_reports_a_missing_credential() {
    let mut fixture = Fixture::new();
    fixture.anthropic = unauthenticated_settings();
    let values = kwargs();
    let failure = fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect_err("a missing credential fails the list");
    assert_eq!(
        failure.error.to_string(),
        "ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN is required for Skills API"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn validate_environment_failure_is_raised_before_the_request() {
    let mut fixture = Fixture::new();
    fixture.anthropic = unauthenticated_settings();
    let values = kwargs();
    fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect_err("a missing credential fails the list");
    assert!(fixture.http.requests.lock().expect("the fake").is_empty());
    assert_eq!(fixture.builds(), 0);
}

#[tokio::test]
async fn validation_failure_preserves_an_invalid_header_error() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: Some(SecretString::from("bad\nkey")),
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, None, None, None, &values);
    let failure = fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect_err("an invalid header value fails the list");
    assert_eq!(
        failure.error.to_string(),
        "the x-api-key header value is not a valid HTTP header value"
    );
    assert!(fixture.http.requests.lock().expect("the fake").is_empty());
}
