#![cfg(test)]

use http::HeaderMap;
use indexmap::IndexMap;

use crate::api::skills::fixtures::{
    CALL_ID, Fixture, SKILL_ID, call_with, kwargs, logging, skill_request, text,
};
use crate::api::skills::get_delete::call::SkillsCallParams;

#[tokio::test]
async fn extra_query_is_accepted_and_not_forwarded() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut extra_query = IndexMap::new();
    extra_query.insert("page".to_owned(), text("2"));
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: Some(extra_query),
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        (requests[0].url.as_str(), requests[0].url.contains("page"),),
        ("https://api.anthropic.com/v1/skills/skill_1", false)
    );
}

#[tokio::test]
async fn extra_query_is_not_forwarded_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut extra_query = IndexMap::new();
    extra_query.insert("page".to_owned(), text("2"));
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: Some(extra_query),
        timeout: None,
        custom_llm_provider: Some("litellm_proxy".to_owned()),
    };
    let hosted = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].skill_id, SKILL_ID);
    assert_eq!(calls[0].litellm_call_id.as_deref(), Some(CALL_ID));
}
