#![cfg(test)]

use http::HeaderMap;

use crate::api::skills::create_fixtures::{
    Fixture, call, call_with, hosted_params, logging, skill_creation,
};
use crate::api::skills::fixtures::{kwargs, sample_skill};
use crate::api::skills::get_delete::call::SkillsCallParams;

#[tokio::test]
async fn create_skill_routing_defaults_to_anthropic() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .create_skill(skill_creation(&call(&values), &mut logging()))
        .await
        .expect("the external create succeeds");
    let creates = fixture.http.creates.lock().expect("the fake");
    assert_eq!(creates.len(), 1);
    assert_eq!(creates[0].provider, "anthropic");
    drop(creates);
    assert_eq!(fixture.builds(), 0);
}

#[tokio::test]
async fn create_skill_hosted_routes_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), None, None, None, &values);
    fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect("the hosted create succeeds");
    assert!(fixture.http.creates.lock().expect("the fake").is_empty());
    assert_eq!(fixture.hosted.calls.lock().expect("the fake").len(), 1);
    assert_eq!(fixture.builds(), 1);
}

#[tokio::test]
async fn create_skill_external_dispatches_through_the_http_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("anthropic".to_owned()),
    };
    let external = call_with(params, None, None, None, &values);
    let skill = fixture
        .service()
        .create_skill(skill_creation(&external, &mut logging()))
        .await
        .expect("the external create succeeds");
    assert_eq!(skill, sample_skill());
    assert_eq!(fixture.hosted.calls.lock().expect("the fake").len(), 0);
}
