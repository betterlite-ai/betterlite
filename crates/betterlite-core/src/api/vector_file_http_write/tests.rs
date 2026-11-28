#![cfg(test)]

mod delete;
mod failures;
mod modes;
mod support;
mod update;

use betterlite_models::decisions::Json;

use self::support::{Events, Fixture};
use crate::api::vector_file_http_write::config::FileWriteConfig;
use crate::api::vector_file_http_write::error::FileWriteError;
use crate::api::vector_file_http_write::service::{WriteCall, WriteDeps, WriteTransport, send};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource};

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

pub(crate) async fn run<'deps, C, S>(
    fixture: &mut Fixture,
    deps: &'deps WriteDeps<'deps, C, S>,
    mode: ClientKind,
    call: &WriteCall<'_>,
    events: &Events,
) -> Result<Json, FileWriteError>
where
    C: FileWriteConfig,
    S: ClientSource<Client: WriteTransport>,
{
    let prepared = fixture.prepare(deps, mode, call, events)?;
    send(prepared).await
}
