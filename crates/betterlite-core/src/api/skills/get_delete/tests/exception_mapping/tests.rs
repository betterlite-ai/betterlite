#![cfg(test)]

use betterlite_error::error::ErrorClass;
use http::HeaderMap;

use crate::api::skills::fixtures::{
    Fixture, SKILL_ID, call, call_error, call_with, kwargs, logging, redactor, skill_request,
};
use crate::api::skills::get_delete::call::SkillsCallParams;
use crate::api::skills::get_delete::error::failure;

#[tokio::test]
async fn exception_mapping_maps_a_provider_status() {
    let fixture = Fixture::failing(404);
    let values = kwargs();
    let failure = fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect_err("the provider error fails the call");
    assert!(matches!(failure.class, ErrorClass::NotFound(_)));
    assert_eq!(
        failure.error.to_string(),
        "the provider answered with HTTP 404: the provider refused"
    );
}

#[tokio::test]
async fn exception_mapping_maps_the_delete_path() {
    let fixture = Fixture::failing(401);
    let values = kwargs();
    let failure = fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect_err("the provider error fails the delete");
    assert!(matches!(failure.class, ErrorClass::Authentication(_)));
}

#[tokio::test]
async fn exception_type_uses_anthropic_mapping() {
    let fixture = Fixture::failing(403);
    let values = kwargs();
    let failure = fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect_err("the provider error fails the call");
    assert!(matches!(failure.class, ErrorClass::PermissionDenied(_)));
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
    let call = call_with(params, SKILL_ID, &values);
    let mapped = failure(call_error(403), &call, &redactor());
    assert!(matches!(mapped.class, ErrorClass::Api(_)));
}

#[test]
fn exception_type_uses_the_anthropic_class_for_the_same_status() {
    let values = kwargs();
    let call = call(SKILL_ID, &values);
    let mapped = failure(call_error(403), &call, &redactor());
    assert!(matches!(mapped.class, ErrorClass::PermissionDenied(_)));
}
