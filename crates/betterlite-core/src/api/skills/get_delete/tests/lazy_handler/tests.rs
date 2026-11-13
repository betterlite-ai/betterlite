#![cfg(test)]

use crate::api::skills::fixtures::{
    Fixture, SKILL_ID, call, call_with, hosted_params, kwargs, logging, skill_request,
};

#[tokio::test]
async fn lazy_handler_builds_once_for_many_hosted_calls() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    let service = fixture.service();
    service
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    service
        .delete_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted delete succeeds");
    service
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the second hosted get succeeds");
    assert_eq!(fixture.builds(), 1);
    assert_eq!(fixture.hosted.calls.lock().expect("the fake").len(), 2);
    assert_eq!(fixture.hosted.deletes.lock().expect("the fake").len(), 1);
}

#[tokio::test]
async fn handler_init_reuses_the_cached_instance() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    let service = fixture.service();
    service
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    service
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the second hosted get succeeds");
    assert_eq!(fixture.builds(), 1);
}

#[tokio::test]
async fn lazy_handler_is_not_built_for_external_calls() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    assert_eq!(fixture.builds(), 0);
}
