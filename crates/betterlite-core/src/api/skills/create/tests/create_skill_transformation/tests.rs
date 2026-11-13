#![cfg(test)]

use betterlite_anthropic::credentials::ANTHROPIC_VERSION;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::skills::create_fixtures::{
    Fixture, call, call_with, logging, map, skill_creation, unauthenticated_settings,
};
use crate::api::skills::fixtures::{extra_headers, header_values, kwargs, text};
use crate::api::skills::get_delete::call::SkillsCallParams;

fn skill_file(name: &str) -> Vec<Json> {
    vec![Json::Object(map(&[("filename", text(name))]))]
}

#[tokio::test]
async fn create_skill_request_carries_the_validated_headers() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(
        header_values(&creates[0].headers, "x-api-key"),
        ["sk-ant-test"]
    );
    assert!(
        creates[0]
            .headers
            .get("x-api-key")
            .expect("the auth header exists")
            .is_sensitive()
    );
    assert_eq!(
        header_values(&creates[0].headers, "anthropic-version"),
        [ANTHROPIC_VERSION]
    );
    assert_eq!(
        header_values(&creates[0].headers, "anthropic-beta"),
        ["skills-2025-10-02"]
    );
    assert_eq!(
        header_values(&creates[0].headers, "content-type"),
        ["application/json"]
    );
}

#[tokio::test]
async fn create_skill_transformation_merges_the_caller_beta_header() {
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
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(
        header_values(&creates[0].headers, "anthropic-beta"),
        ["custom-beta", "skills-2025-10-02"]
    );
}

#[tokio::test]
async fn create_skill_transformation_reports_a_missing_credential() {
    let mut fixture = Fixture::new();
    fixture.anthropic = unauthenticated_settings();
    let values = kwargs();
    let failure = fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut logging()))
        .await
        .expect_err("a missing credential fails");
    assert_eq!(
        failure.error.to_string(),
        "ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN is required for Skills API"
    );
    assert!(fixture.http.creates.lock().expect("the fake").is_empty());
}

#[tokio::test]
async fn create_skill_transformation_prefers_the_call_api_key() {
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
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(
        header_values(&creates[0].headers, "x-api-key"),
        ["sk-ant-call"]
    );
}

#[tokio::test]
async fn create_skill_request_carries_the_body_to_the_handler() {
    let fixture = Fixture::new();
    let given = skill_file("SKILL.md");
    let mut extra_body = IndexMap::new();
    extra_body.insert("description".to_owned(), text("what it does"));
    let values = kwargs();
    let request = call_with(
        SkillsCallParams::default(),
        Some("Release notes"),
        Some(&given),
        Some(&extra_body),
        &values,
    );
    fixture
        .service()
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(
        creates[0].body.display_title.as_deref(),
        Some("Release notes")
    );
    assert_eq!(creates[0].body.files, Some(given));
    assert_eq!(creates[0].body.extra, extra_body);
}

#[tokio::test]
async fn create_skill_request_drops_null_extra_body_values() {
    let fixture = Fixture::new();
    let mut extra_body = IndexMap::new();
    extra_body.insert("description".to_owned(), Json::Null);
    extra_body.insert("metadata".to_owned(), Json::Object(map(&[])));
    let values = kwargs();
    let request = call_with(
        SkillsCallParams::default(),
        None,
        None,
        Some(&extra_body),
        &values,
    );
    fixture
        .service()
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(
        creates[0].body.extra,
        map(&[("metadata", Json::Object(map(&[])))])
    );
}
