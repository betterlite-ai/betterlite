use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use betterlite_azure::azure_files::client::Connection;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{
    create_file, delete_file, file_content, list_files, retrieve_file,
};
use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_openai::files::records::{ExpiresAfter, FileContentRequest, FileCreate, FileUpload};
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::runtime::Builder as RuntimeBuilder;

use betterlite_oracle_helper_api::error::HelperError;

use super::fixtures::materializer::{Materializer, params, settings, supplied};
use super::fixtures::sources::{Cache, Sources};
use super::fixtures::transport::{Log, Recorded};
use super::input::{Action, Step};
use super::reports;

pub(super) fn run(step: &Step) -> Result<Value, Box<HelperError>> {
    present(step)?;
    let calls: Log = Arc::default();
    let transport = Recorded::new(step.reply.as_ref(), Arc::clone(&calls));
    let materializer = Materializer {
        builder: step.builder,
    };
    let sources = Sources;
    let endpoints = EndpointSettings::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let mut cache = Cache;
    let mut refresh = RefreshProviderCache::new(&sources);
    let connection = Connection {
        settings: settings(step),
        params: params(step),
        provider: None,
        supplied: step.client.map(supplied),
        is_async: step.is_async,
        endpoints: &endpoints,
        environment: &environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer: &materializer,
    };
    let result = RuntimeBuilder::new_current_thread()
        .build()
        .expect("the helper runtime builds")
        .block_on(perform(step, connection, &transport));
    drop(transport);
    let calls = Arc::into_inner(calls)
        .expect("the transport holds the only log")
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let request = calls.first().map_or(Value::Null, reports::request);
    Ok(match result {
        Ok(value) => json!({ "request": request, "result": value, "error": Value::Null }),
        Err(error) => {
            json!({ "request": request, "result": Value::Null, "error": reports::raised(&error) })
        }
    })
}

async fn perform(
    step: &Step,
    connection: Connection<'_, '_, Materializer>,
    transport: &Recorded,
) -> Result<Value, AzureFilesError> {
    match step.action {
        Action::Client => connection.open().map(|client| reports::client(&client)),
        Action::Create => create_file(connection, transport, &create(step))
            .await
            .map(|object| reports::object(&object)),
        Action::Content => {
            let request = FileContentRequest {
                file_id: file_id(step).to_owned(),
                extra_headers: IndexMap::new(),
                extra_body: IndexMap::new(),
                timeout: None,
            };
            file_content(connection, transport, &request)
                .await
                .map(|content| reports::content(&content))
        }
        Action::Retrieve => retrieve_file(connection, transport, file_id(step))
            .await
            .map(|object| reports::object(&object)),
        Action::List => list_files(connection, transport, step.purpose.as_deref())
            .await
            .map(|page| reports::page(&page)),
        Action::Delete => delete_file(connection, transport, file_id(step))
            .await
            .map(|deleted| reports::deleted(&deleted)),
    }
}

// The SDK client's multipart call, with the file bytes LiteLLM's CreateFileRequest carries
fn create(step: &Step) -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: None,
            content_type: None,
            data: step.content.clone().unwrap_or_default().into_bytes(),
        },
        purpose: step
            .create_purpose
            .expect("the step carries a create purpose"),
        expires_after: step.expires_after.map(|seconds| ExpiresAfter { seconds }),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

fn file_id(step: &Step) -> &str {
    step.file_id.as_deref().expect("the step carries a file id")
}

// the required fields are checked before the runtime starts, so a bad case is an input error
fn present(step: &Step) -> Result<(), Box<HelperError>> {
    let field = match step.action {
        Action::Create if step.content.is_none() => "content",
        Action::Create if step.create_purpose.is_none() => "create_purpose",
        Action::Content | Action::Retrieve | Action::Delete if step.file_id.is_none() => "file_id",
        _ => return Ok(()),
    };
    Err(Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: format!("an azure_files step needs {field}"),
    }))
}
