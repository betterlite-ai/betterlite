#![cfg(test)]

mod create;
mod failures;
mod headers;
mod list;
mod modes;
mod retrieve;
mod support;

use betterlite_models::decisions::Json;

use self::support::{Events, Fixture};
use crate::api::vector_store_http_create_read::config::CreateReadConfig;
use crate::api::vector_store_http_create_read::error::CreateReadError;
use crate::api::vector_store_http_create_read::service::{CreateReadCall, CreateReadDeps, send};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource, WriteTransport};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

async fn run<'deps, C, S>(
    fixture: &mut Fixture,
    deps: &'deps CreateReadDeps<'deps, C, S>,
    mode: ClientKind,
    call: &CreateReadCall<'_>,
    events: &Events,
) -> Result<Json, CreateReadError>
where
    C: CreateReadConfig,
    S: ClientSource<Client: WriteTransport>,
{
    let prepared = fixture.prepare(deps, mode, call, events)?;
    send(prepared).await
}
