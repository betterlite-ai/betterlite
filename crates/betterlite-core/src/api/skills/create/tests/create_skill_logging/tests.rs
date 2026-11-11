#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::skills::create_fixtures::{
    Fixture, call, call_with, hosted_params, logging, map, skill_creation,
};
use crate::api::skills::fixtures::{CALL_ID, kwargs, text};
use crate::api::skills::get_delete::call::SkillsCallParams;

fn skill_file(name: &str) -> Vec<Json> {
    vec![Json::Object(map(&[("filename", text(name))]))]
}

#[tokio::test]
async fn create_skill_logging_records_the_request_body() {
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
    let mut state = logging();
    fixture
        .service()
        .create_skill(skill_creation(&request, &mut state))
        .await
        .expect("the external create succeeds");
    assert_eq!(
        state.optional_params,
        map(&[
            ("display_title", text("Release notes")),
            ("files", Json::List(given)),
            ("description", text("what it does")),
        ])
    );
    assert_eq!(state.custom_llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(
        state.model_call_details.get("display_title"),
        Some(&text("Release notes"))
    );
}

#[tokio::test]
async fn logging_records_the_call_id_from_the_call() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut state = logging();
    fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut state))
        .await
        .expect("the external create succeeds");
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&text(CALL_ID))
    );
}

#[tokio::test]
async fn create_skill_logging_omits_absent_fields() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut state = logging();
    fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut state))
        .await
        .expect("the external create succeeds");
    assert!(state.optional_params.is_empty());
}

#[tokio::test]
async fn pre_call_logging_uses_litellm_proxy_for_hosted() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), Some("Hosted title"), None, None, &values);
    let mut state = logging();
    fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut state))
        .await
        .expect("the hosted create succeeds");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("litellm_proxy"));
    assert_eq!(
        state.optional_params.get("display_title"),
        Some(&text("Hosted title"))
    );
}
