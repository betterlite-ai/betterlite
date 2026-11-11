#![cfg(test)]

use futures_util::future::join;

use crate::api::skills::create_fixtures::{
    Fixture, call_with, hosted_params, logging, skill_creation,
};
use crate::api::skills::fixtures::{kwargs, metadata_with, object, text};

#[tokio::test]
async fn acreate_skill_concurrent_calls_keep_their_own_metadata() {
    let fixture = Fixture::new();
    let first_metadata = object(&[("requester", text("first"))]);
    let second_metadata = object(&[("requester", text("second"))]);
    let first_kwargs_metadata = metadata_with(first_metadata.clone());
    let second_kwargs_metadata = metadata_with(second_metadata.clone());
    let mut first_values = kwargs();
    first_values.insert("metadata".to_owned(), first_kwargs_metadata.clone());
    let mut second_values = kwargs();
    second_values.insert("metadata".to_owned(), second_kwargs_metadata.clone());
    let first_call = call_with(hosted_params(), None, None, None, &first_values);
    let second_call = call_with(hosted_params(), None, None, None, &second_values);
    let service = fixture.service();
    let mut first_state = logging();
    let mut second_state = logging();
    let (first, second) = join(
        service.create_skill(skill_creation(&first_call, &mut first_state)),
        service.create_skill(skill_creation(&second_call, &mut second_state)),
    )
    .await;
    first.expect("the first hosted create succeeds");
    second.expect("the second hosted create succeeds");
    assert_eq!(
        first_state.litellm_params.get("metadata"),
        Some(&first_kwargs_metadata)
    );
    assert_eq!(
        second_state.litellm_params.get("metadata"),
        Some(&second_kwargs_metadata)
    );
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].principal, Some(first_metadata));
    assert_eq!(calls[1].principal, Some(second_metadata));
}

#[tokio::test]
async fn acreate_skill_returns_the_hosted_answer() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), None, None, None, &values);
    let skill = fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect("the hosted create succeeds");
    assert_eq!(skill.id, "skill_1");
    assert_eq!(fixture.builds(), 1);
}
