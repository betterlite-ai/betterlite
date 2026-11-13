#![cfg(test)]

use betterlite_error::error::ErrorClass;

use crate::api::skills::fixtures::kwargs;
use crate::api::skills::list::error::ListSkillsError;
use crate::api::skills::list_fixtures::{Fixture, call_with, listing, logging, params_for};

#[test]
fn list_skill_error_message_matches_litellm() {
    let error = ListSkillsError::Unsupported {
        provider: "openai".to_owned(),
    };
    assert_eq!(error.to_string(), "LIST skills is not supported for openai");
}

#[tokio::test]
async fn unsupported_provider_reports_the_litellm_message() {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(params_for("openai"), None, None, None, &values);
    let failure = fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect_err("the unsupported provider fails the call");
    assert_eq!(
        failure.error.to_string(),
        "LIST skills is not supported for openai"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.http.requests.lock().expect("the fake").is_empty());
    assert!(fixture.hosted.calls.lock().expect("the fake").is_empty());
    assert_eq!(fixture.builds(), 0);
}

#[tokio::test]
async fn unsupported_provider_is_reported_for_any_other_provider() {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(params_for("mistral"), None, None, None, &values);
    let failure = fixture
        .service()
        .list_skills(listing(&request, &mut logging()))
        .await
        .expect_err("the unsupported provider fails the call");
    assert_eq!(
        failure.error.to_string(),
        "LIST skills is not supported for mistral"
    );
}
