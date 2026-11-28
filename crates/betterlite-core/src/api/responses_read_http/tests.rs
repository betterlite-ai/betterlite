#![cfg(test)]

mod errors;
mod query;
mod retrieve;
mod selection;
mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use self::support::{Events, Fixture};
use crate::api::responses_read_http::config::{ReadConfig, ReadParams};
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::{
    ClientKind, ReadCall, ReadDeps, ReadOutcome, ReadTarget, ReadTransport, send,
};
use crate::api::vector_store_http_write::service::ClientSource;

const fn target<'call>(
    response_id: &'call str,
    params: &'call ReadParams,
    extra_headers: Option<&'call BTreeMap<String, String>>,
    timeout: Option<Duration>,
) -> ReadTarget<'call> {
    ReadTarget {
        response_id,
        params,
        extra_headers,
        timeout,
    }
}

async fn run<'deps, C, S>(
    fixture: &mut Fixture,
    deps: &'deps ReadDeps<'deps, C, S>,
    mode: ClientKind,
    call: &ReadCall<'_>,
    events: &Events,
) -> Result<ReadOutcome, ReadError>
where
    C: ReadConfig,
    S: ClientSource<Client: ReadTransport>,
{
    let prepared = fixture.prepare(deps, mode, call, events)?;
    send(prepared).await
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn retrieve_headers_and_response(#[case] mode: ClientKind) {
    retrieve::headers_and_response(mode).await;
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn input_items_query(#[case] mode: ClientKind) {
    query::input_items(mode).await;
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn read_failure_boundary(#[case] mode: ClientKind) {
    errors::failure_boundary(mode).await;
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn read_client_selection(#[case] mode: ClientKind) {
    selection::client_selection(mode).await;
}
