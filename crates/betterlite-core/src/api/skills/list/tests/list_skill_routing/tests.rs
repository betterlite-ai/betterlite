#![cfg(test)]

use crate::api::skills::fixtures::kwargs;
use crate::api::skills::list_fixtures::{
    Fixture, call, call_with, hosted_params, listing, logging, params_for, sample_list_response,
};

#[tokio::test]
async fn list_skill_routing_defaults_to_anthropic() {
    let fixture = Fixture::new();
    let values = kwargs();
    let response = fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect("the external list succeeds");
    assert_eq!(response, sample_list_response());
    let requests = fixture.http.requests.lock().expect("the fake");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider, "anthropic");
    drop(requests);
    assert_eq!(fixture.builds(), 0);
}

#[tokio::test]
async fn list_skill_hosted_routes_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), None, None, None, &values);
    let response = fixture
        .service()
        .list_skills(listing(&hosted, &mut logging()))
        .await
        .expect("the hosted list succeeds");
    assert_eq!(response, sample_list_response());
    assert!(fixture.http.requests.lock().expect("the fake").is_empty());
    assert_eq!(fixture.hosted.calls.lock().expect("the fake").len(), 1);
    assert_eq!(fixture.builds(), 1);
}

#[tokio::test]
async fn list_skill_external_dispatches_through_the_http_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let external = call_with(params_for("anthropic"), None, None, None, &values);
    let response = fixture
        .service()
        .list_skills(listing(&external, &mut logging()))
        .await
        .expect("the external list succeeds");
    assert_eq!(response, sample_list_response());
    assert_eq!(fixture.http.requests.lock().expect("the fake").len(), 1);
    assert!(fixture.hosted.calls.lock().expect("the fake").is_empty());
}
