#![cfg(test)]

use secrecy::ExposeSecret as _;

use crate::database_refresh::access::RefreshTrigger;
use crate::database_refresh_loop::tests::fixtures::postgres::{
    assert_published, callback_error_case, candidate_failure_case, close_case, closed, loop_on,
    seeded_case, writer_endpoint_from,
};

mod behavior;
mod corpus;
mod fixtures;
mod lifecycle;

#[tokio::test]
async fn postgres_pinned_rows() {
    let case = seeded_case().await;
    let endpoint = writer_endpoint_from(case.url.expose_secret());
    let setup = loop_on(case, endpoint, None).await;

    RefreshTrigger::refresh(setup.built.port.as_ref())
        .await
        .expect("the refresh replaces the pool");

    assert_published(&setup).await;
    closed(setup.initial.as_ref()).await;
    Box::pin(close_case(setup)).await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    Box::pin(candidate_failure_case()).await;
    Box::pin(callback_error_case()).await;
}
