#![cfg(test)]

use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::api::skills::create_fixtures::{Fixture, call_with, logging, skill_creation};
use crate::api::skills::fixtures::{kwargs, text};
use crate::api::skills::get_delete::call::SkillsCallParams;

fn query() -> IndexMap<String, Json> {
    let mut extra_query = IndexMap::new();
    extra_query.insert("page".to_owned(), text("2"));
    extra_query
}

#[tokio::test]
async fn extra_query_is_accepted_and_not_forwarded() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: Some(query()),
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, None, None, None, &values);
    fixture
        .service()
        .create_skill(skill_creation(&request, &mut logging()))
        .await
        .expect("the external create succeeds");
    let (url, extra) = {
        let creates = fixture.http.creates.lock().expect("the fake");
        (creates[0].url.clone(), creates[0].body.extra.clone())
    };
    assert_eq!(url, "https://api.anthropic.com/v1/skills");
    assert!(!url.contains("page"));
    assert!(extra.is_empty());
}

#[tokio::test]
async fn extra_query_is_not_forwarded_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: Some(query()),
        timeout: None,
        custom_llm_provider: Some("litellm_proxy".to_owned()),
    };
    let hosted = call_with(params, Some("Title"), None, None, &values);
    fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect("the hosted create succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].display_title.as_deref(), Some("Title"));
    assert_eq!(fixture.builds(), 1);
}
