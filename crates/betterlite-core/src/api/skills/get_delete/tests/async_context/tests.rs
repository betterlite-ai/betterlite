#![cfg(test)]

use futures_util::future::join;

use crate::api::skills::fixtures::{
    Fixture, SKILL_ID, call, call_with, hosted_params, kwargs, logging, metadata_with, object,
    skill_request, text,
};

#[tokio::test]
async fn async_context_concurrent_calls_keep_their_own_metadata() {
    let fixture = Fixture::new();
    let first_metadata = object(&[("user", text("first"))]);
    let second_metadata = object(&[("user", text("second"))]);
    let mut first_values = kwargs();
    first_values.insert("metadata".to_owned(), first_metadata.clone());
    let mut second_values = kwargs();
    second_values.insert("metadata".to_owned(), second_metadata.clone());
    let first_call = call(SKILL_ID, &first_values);
    let second_call = call(SKILL_ID, &second_values);
    let service = fixture.service();
    let mut first_state = logging();
    let mut second_state = logging();
    let (first, second) = join(
        service.get_skill(skill_request(&first_call, &mut first_state)),
        service.get_skill(skill_request(&second_call, &mut second_state)),
    )
    .await;
    first.expect("the first call succeeds");
    second.expect("the second call succeeds");
    assert_eq!(
        first_state.litellm_params.get("metadata"),
        Some(&first_metadata)
    );
    assert_eq!(
        second_state.litellm_params.get("metadata"),
        Some(&second_metadata)
    );
}

#[tokio::test]
async fn async_context_hosted_get_receives_the_principal() {
    let fixture = Fixture::new();
    let principal = object(&[("user_id", text("user-1"))]);
    let mut values = kwargs();
    values.insert("metadata".to_owned(), metadata_with(principal.clone()));
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].principal, Some(principal));
}
