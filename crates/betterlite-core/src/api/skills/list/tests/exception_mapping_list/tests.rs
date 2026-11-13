#![cfg(test)]

use betterlite_error::error::ErrorClass;

use crate::api::skills::fixtures::{kwargs, redactor};
use crate::api::skills::get_delete::error::SkillsCallError;
use crate::api::skills::get_delete::handler::SkillsHandlerError;
use crate::api::skills::list::error::{ListSkillsError, failure};
use crate::api::skills::list_fixtures::{
    Fixture, call, call_with, hosted_params, listing, logging,
};

fn call_error(status_code: u16) -> SkillsCallError {
    SkillsCallError::Handler(SkillsHandlerError::Provider {
        status_code,
        message: "the provider refused".to_owned(),
        status_code_is_synthesized: false,
        body_code: None,
        body_message: None,
    })
}

#[tokio::test]
async fn exception_mapping_list_maps_a_provider_status() {
    let fixture = Fixture::failing(404);
    let values = kwargs();
    let failure = fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
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
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect_err("the provider error fails the call");
    assert!(matches!(failure.class, ErrorClass::PermissionDenied(_)));
}

#[test]
fn exception_type_uses_the_litellm_proxy_family_mapping() {
    let values = kwargs();
    let request = call_with(hosted_params(), None, None, None, &values);
    let mapped = failure(
        ListSkillsError::Call(call_error(403)),
        &request,
        &redactor(),
    );
    assert!(matches!(mapped.class, ErrorClass::Api(_)));
}

#[test]
fn exception_type_uses_the_anthropic_class_for_the_same_status() {
    let values = kwargs();
    let request = call(&values);
    let mapped = failure(
        ListSkillsError::Call(call_error(403)),
        &request,
        &redactor(),
    );
    assert!(matches!(mapped.class, ErrorClass::PermissionDenied(_)));
}
