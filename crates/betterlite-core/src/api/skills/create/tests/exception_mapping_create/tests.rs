#![cfg(test)]

use betterlite_error::error::ErrorClass;
use http::HeaderMap;

use crate::api::skills::create::error::{CreateSkillError, failure};
use crate::api::skills::create_fixtures::{Fixture, call, call_with, logging, skill_creation};
use crate::api::skills::fixtures::{kwargs, redactor};
use crate::api::skills::get_delete::call::SkillsCallParams;
use crate::api::skills::get_delete::error::SkillsCallError;
use crate::api::skills::get_delete::handler::SkillsHandlerError;

fn handler_error(status_code: u16) -> CreateSkillError {
    CreateSkillError::Call(SkillsCallError::Handler(SkillsHandlerError::Provider {
        status_code,
        message: "the provider refused".to_owned(),
        status_code_is_synthesized: false,
        body_code: None,
        body_message: None,
    }))
}

#[tokio::test]
async fn exception_mapping_maps_a_provider_status() {
    let fixture = Fixture::failing(404);
    let values = kwargs();
    let failure = fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut logging()))
        .await
        .expect_err("the provider error fails the call");
    assert!(matches!(failure.class, ErrorClass::NotFound(_)));
    assert_eq!(
        failure.error.to_string(),
        "the provider answered with HTTP 404: the provider refused"
    );
}

#[tokio::test]
async fn exception_type_uses_anthropic_mapping() {
    let fixture = Fixture::failing(403);
    let values = kwargs();
    let failure = fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut logging()))
        .await
        .expect_err("the provider error fails the call");
    assert!(matches!(failure.class, ErrorClass::PermissionDenied(_)));
}

#[test]
fn exception_type_uses_the_anthropic_class_for_the_same_status() {
    let values = kwargs();
    let mapped = failure(handler_error(403), &call(&values), &redactor());
    assert!(matches!(mapped.class, ErrorClass::PermissionDenied(_)));
}

#[test]
fn exception_type_uses_the_litellm_proxy_family_mapping() {
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("litellm_proxy".to_owned()),
    };
    let call = call_with(params, None, None, None, &values);
    let mapped = failure(handler_error(403), &call, &redactor());
    assert!(matches!(mapped.class, ErrorClass::Api(_)));
}

#[tokio::test]
async fn create_skill_exception_type_maps_a_transport_failure() {
    let fixture = Fixture::failing(401);
    let values = kwargs();
    let failure = fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut logging()))
        .await
        .expect_err("the provider error fails the call");
    assert!(matches!(failure.class, ErrorClass::Authentication(_)));
}
