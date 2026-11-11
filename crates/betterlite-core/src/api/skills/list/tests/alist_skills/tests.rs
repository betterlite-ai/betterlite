#![cfg(test)]

use futures_util::future::join;

use crate::api::skills::fixtures::{kwargs, metadata_with, object, text};
use crate::api::skills::list_fixtures::{
    Fixture, call, call_with, hosted_params, listing, logging, sample_list_response,
};

#[tokio::test]
async fn alist_skills_concurrent_calls_keep_their_own_metadata() {
    let fixture = Fixture::new();
    let first_metadata = object(&[("user", text("first"))]);
    let second_metadata = object(&[("user", text("second"))]);
    let mut first_values = kwargs();
    first_values.insert("metadata".to_owned(), first_metadata.clone());
    let mut second_values = kwargs();
    second_values.insert("metadata".to_owned(), second_metadata.clone());
    let first_call = call(&first_values);
    let second_call = call(&second_values);
    let service = fixture.service();
    let mut first_state = logging();
    let mut second_state = logging();
    let (first, second) = join(
        service.list_skills(listing(&first_call, &mut first_state)),
        service.list_skills(listing(&second_call, &mut second_state)),
    )
    .await;
    first.expect("the first list succeeds");
    second.expect("the second list succeeds");
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
async fn alist_skills_returns_the_external_answer() {
    let fixture = Fixture::new();
    let values = kwargs();
    let response = fixture
        .service()
        .list_skills(listing(&call(&values), &mut logging()))
        .await
        .expect("the external list succeeds");
    assert_eq!(response, sample_list_response());
}

#[tokio::test]
async fn alist_skills_hosted_receives_the_principal() {
    let fixture = Fixture::new();
    let principal = object(&[("user_id", text("user-1"))]);
    let mut values = kwargs();
    values.insert("metadata".to_owned(), metadata_with(principal.clone()));
    let hosted = call_with(hosted_params(), None, None, None, &values);
    fixture
        .service()
        .list_skills(listing(&hosted, &mut logging()))
        .await
        .expect("the hosted list succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].principal, Some(principal));
}
