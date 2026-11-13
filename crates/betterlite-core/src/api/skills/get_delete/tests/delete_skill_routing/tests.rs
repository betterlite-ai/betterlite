#![cfg(test)]

use crate::api::skills::fixtures::{
    Fixture, SKILL_ID, call, call_with, hosted_params, kwargs, logging, sample_deleted,
    skill_request,
};

#[tokio::test]
async fn delete_skill_routing_defaults_to_anthropic() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external delete succeeds");
    let requests = fixture.http.delete_requests.lock().expect("the fake");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider, "anthropic");
}

#[tokio::test]
async fn delete_skill_hosted_routes_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    fixture
        .service()
        .delete_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted delete succeeds");
    assert!(
        fixture
            .http
            .delete_requests
            .lock()
            .expect("the fake")
            .is_empty()
    );
    assert_eq!(fixture.hosted.deletes.lock().expect("the fake").len(), 1);
}

#[tokio::test]
async fn delete_skill_external_returns_the_deleted_skill() {
    let fixture = Fixture::new();
    let values = kwargs();
    let deleted = fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external delete succeeds");
    assert_eq!(deleted, sample_deleted());
}
