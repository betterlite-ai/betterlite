#![cfg(test)]

use crate::api::skills::create_fixtures::{
    Fixture, call, call_with, logging, params_with_base, skill_creation,
};
use crate::api::skills::fixtures::kwargs;

#[tokio::test]
async fn create_skill_url_targets_the_skills_endpoint() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(creates[0].url, "https://api.anthropic.com/v1/skills");
}

#[tokio::test]
async fn fallback_base_has_a_single_v1_segment() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(creates[0].url.matches("/v1/").count(), 1);
}

#[tokio::test]
async fn create_skill_url_prefers_the_call_api_base() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = params_with_base(Some("https://gateway.example"));
    let request = call_with(params, None, None, None, &values);
    fixture
        .service()
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(creates[0].url, "https://gateway.example/v1/skills");
}

#[tokio::test]
async fn url_construction_ignores_an_empty_call_api_base() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = params_with_base(Some(""));
    let request = call_with(params, None, None, None, &values);
    fixture
        .service()
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(creates[0].url, "https://api.anthropic.com/v1/skills");
}
