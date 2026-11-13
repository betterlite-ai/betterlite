#![cfg(test)]

use crate::api::skills::fixtures::{
    Fixture, SKILL_ID, call, call_with, extra_headers, hosted_params, kwargs, logging,
    sample_skill, skill_request,
};
use crate::api::skills::get_delete::call::SkillsCallParams;

#[tokio::test]
async fn get_skill_routing_defaults_to_anthropic() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider, "anthropic");
    drop(requests);
    assert_eq!(fixture.builds(), 0);
}

#[tokio::test]
async fn get_skill_hosted_routes_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    assert!(
        fixture
            .http
            .get_requests
            .lock()
            .expect("the fake")
            .is_empty()
    );
    assert_eq!(fixture.hosted.calls.lock().expect("the fake").len(), 1);
    assert_eq!(fixture.builds(), 1);
}

#[tokio::test]
async fn get_skill_external_dispatches_through_the_http_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: extra_headers("x-request-id", "request-1"),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("anthropic".to_owned()),
    };
    let request = call_with(params, SKILL_ID, &values);
    let skill = fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    assert_eq!(skill, sample_skill());
    assert_eq!(fixture.hosted.calls.lock().expect("the fake").len(), 0);
}
