#![cfg(test)]

mod create_list;
mod credentials;
mod failures;
mod modes;
mod support;

use betterlite_models::decisions::Json;

use self::support::{Events, Fixture};
use crate::api::vector_file_http_create_list::config::FilesConfig;
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_file_http_create_list::service::{
    FileCall, FileDeps, FileTransport, PreparedFile, send,
};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

const FILE: &str = r#"{"id":"file_a","object":"vector_store.file","status":"completed"}"#;
const LIST: &str = r#"{"object":"list","data":[],"has_more":false}"#;

async fn run<'deps, C, S>(
    fixture: &mut Fixture,
    deps: &'deps FileDeps<'deps, C, S>,
    mode: ClientKind,
    call: &FileCall<'_>,
    events: &Events,
) -> Result<Json, FileTransportError>
where
    C: FilesConfig,
    S: ClientSource<Client: FileTransport>,
{
    let prepared: PreparedFile<'deps, C, S> = fixture.prepare(deps, mode, call, events)?;
    send(prepared).await
}
