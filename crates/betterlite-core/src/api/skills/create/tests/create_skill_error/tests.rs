#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::api::skills::create::error::CreateSkillError;
use crate::api::skills::create_fixtures::{
    Fixture, call_with, hosted_params, logging, params_with_base, skill_creation,
};
use crate::api::skills::fixtures::kwargs;
use crate::api::skills::get_delete::call::SkillsCallParams;

fn params_for(provider: &str) -> SkillsCallParams {
    SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some(provider.to_owned()),
    }
}

#[test]
fn create_skill_error_message_matches_litellm() {
    let error = CreateSkillError::Unsupported {
        provider: "openai".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "CREATE skill is not supported for openai"
    );
}

#[tokio::test]
async fn unsupported_provider_reports_the_litellm_message() {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(params_for("openai"), None, None, None, &values);
    let failure = fixture
        .service()
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect_err("the unsupported provider fails the call");
    assert_eq!(
        failure.error.to_string(),
        "CREATE skill is not supported for openai"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.http.creates.lock().expect("the fake").is_empty());
    assert!(fixture.hosted.calls.lock().expect("the fake").is_empty());
    assert_eq!(fixture.builds(), 0);
}

#[tokio::test]
async fn create_skill_error_names_the_requested_provider() {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(params_for("mistral"), None, None, None, &values);
    let failure = fixture
        .service()
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect_err("the unsupported provider fails the call");
    assert_eq!(
        failure.error.to_string(),
        "CREATE skill is not supported for mistral"
    );
}

#[tokio::test]
async fn an_anthropic_call_is_supported() {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(params_with_base(None), None, None, None, &values);
    fixture
        .service()
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect("anthropic supports the create endpoint");
}

#[tokio::test]
async fn wrong_typed_extra_body_display_title_is_a_bad_request() {
    let fixture = Fixture::new();
    let mut extra_body = IndexMap::new();
    extra_body.insert("display_title".to_owned(), Json::Bool(true));
    let values = kwargs();
    let request = call_with(
        SkillsCallParams::default(),
        None,
        None,
        Some(&extra_body),
        &values,
    );
    let failure = fixture
        .service()
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect_err("a boolean title is not declared");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a wrong-typed field is a 400, not a provider error");
    };
    assert_eq!(details.body_param.as_deref(), Some("display_title"));
    assert_eq!(
        details.message,
        "the display_title field does not have the declared type"
    );
    assert!(fixture.http.creates.lock().expect("the fake").is_empty());
}

#[tokio::test]
async fn hosted_non_string_description_is_a_bad_request() {
    let fixture = Fixture::new();
    let mut values = kwargs();
    values.insert("description".to_owned(), Json::Bool(true));
    let hosted = call_with(hosted_params(), None, None, None, &values);
    let failure = fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect_err("a boolean description is not declared");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a wrong-typed field is a 400, not a provider error");
    };
    assert_eq!(details.body_param.as_deref(), Some("description"));
    assert_eq!(
        details.message,
        "the description field does not have the declared type"
    );
    assert!(fixture.hosted.calls.lock().expect("the fake").is_empty());
}
