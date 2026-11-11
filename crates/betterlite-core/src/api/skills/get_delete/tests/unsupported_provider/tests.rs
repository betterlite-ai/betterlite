#![cfg(test)]

use betterlite_error::error::ErrorClass;
use http::HeaderMap;

use crate::api::skills::fixtures::{Fixture, SKILL_ID, call_with, kwargs, logging, skill_request};
use crate::api::skills::get_delete::call::SkillsCallParams;
use crate::api::skills::get_delete::error::{SkillAction, SkillsCallError};

#[test]
fn get_skill_error_message_matches_litellm() {
    let error = SkillsCallError::Unsupported {
        action: SkillAction::Get,
        provider: "openai".to_owned(),
    };
    assert_eq!(error.to_string(), "GET skill is not supported for openai");
}

#[test]
fn delete_skill_error_message_matches_litellm() {
    let error = SkillsCallError::Unsupported {
        action: SkillAction::Delete,
        provider: "openai".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "DELETE skill is not supported for openai"
    );
}

#[tokio::test]
async fn unsupported_provider_reports_the_litellm_message() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("openai".to_owned()),
    };
    let request = call_with(params, SKILL_ID, &values);
    let failure = fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect_err("the unsupported provider fails the call");
    assert_eq!(
        failure.error.to_string(),
        "GET skill is not supported for openai"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn unsupported_provider_delete_reports_the_litellm_message() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("mistral".to_owned()),
    };
    let request = call_with(params, SKILL_ID, &values);
    let failure = fixture
        .service()
        .delete_skill(skill_request(&request, &mut logging()))
        .await
        .expect_err("the unsupported provider fails the delete");
    assert_eq!(
        failure.error.to_string(),
        "DELETE skill is not supported for mistral"
    );
}
