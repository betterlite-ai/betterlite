use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use betterlite_azure::azure_files::client::Connection;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{Operation, Reply, call};
use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_openai::files::records::{ExpiresAfter, FileContentRequest, FileCreate, FileUpload};
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::runtime::Builder as RuntimeBuilder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use super::fixtures::materializer::{Materializer, params, settings, supplied};
use super::fixtures::sources::{Cache, Sources};
use super::fixtures::transport::{Log, Recorded};
use super::input::{Action, Step};
use super::reports;

// the request an action sends, with every field that action needs read once and the rest left behind
enum Checked<'step> {
    Client,
    Create(FileCreate),
    Content(FileContentRequest),
    Retrieve(&'step str),
    List(Option<&'step str>),
    Delete(&'step str),
}

pub(super) fn run(step: &Step) -> Result<Value, Box<HelperError>> {
    let checked = check(step)?;
    let calls: Log = Arc::default();
    let transport = Recorded::new(step.reply.as_ref(), Arc::clone(&calls))?;
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
        .or_value_error()?
        .block_on(perform(&checked, connection, &transport));
    drop(transport);
    let request = calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .map_or(Value::Null, reports::request);
    Ok(match result {
        Ok(value) => json!({ "request": request, "result": value, "error": Value::Null }),
        Err(error) => {
            json!({ "request": request, "result": Value::Null, "error": reports::raised(&error) })
        }
    })
}

async fn perform(
    checked: &Checked<'_>,
    connection: Connection<'_, '_, Materializer>,
    transport: &Recorded,
) -> Result<Value, AzureFilesError> {
    let operation = match checked {
        Checked::Client => return connection.open().map(|client| reports::client(&client)),
        Checked::Create(data) => Operation::Create(data),
        Checked::Content(request) => Operation::Content(request),
        Checked::Retrieve(file_id) => Operation::Retrieve(*file_id),
        Checked::List(purpose) => Operation::List(*purpose),
        Checked::Delete(file_id) => Operation::Delete(*file_id),
    };
    match call(connection, transport, operation).await? {
        Reply::Object(object) => Ok(reports::object(&object)),
        Reply::Content(binary) => Ok(reports::content(&binary)),
        Reply::Page(page) => Ok(reports::page(&page)),
        Reply::Deleted(deleted) => Ok(json!({
            "id": &deleted.id, "object": deleted.object, "deleted": deleted.deleted,
        })),
    }
}

// every field an action needs is read before the runtime starts, so a bad case fails with its field name
fn check(step: &Step) -> Result<Checked<'_>, Box<HelperError>> {
    match step.action {
        Action::Client => Ok(Checked::Client),
        Action::Create => {
            // the SDK client's multipart call, with the file bytes LiteLLM's CreateFileRequest carries
            let content = step.content.clone().ok_or_else(|| invalid("content"))?;
            let purpose = step
                .create_purpose
                .ok_or_else(|| invalid("create_purpose"))?;
            Ok(Checked::Create(FileCreate {
                file: FileUpload {
                    filename: None,
                    content_type: None,
                    data: content.into_bytes(),
                },
                purpose,
                expires_after: step.expires_after.map(|seconds| ExpiresAfter { seconds }),
                extra_headers: IndexMap::new(),
                extra_body: IndexMap::new(),
                timeout: None,
            }))
        }
        Action::Content => Ok(Checked::Content(FileContentRequest {
            file_id: file_id(step)?.to_owned(),
            extra_headers: IndexMap::new(),
            extra_body: IndexMap::new(),
            timeout: None,
        })),
        Action::Retrieve => Ok(Checked::Retrieve(file_id(step)?)),
        Action::List => Ok(Checked::List(step.purpose.as_deref())),
        Action::Delete => Ok(Checked::Delete(file_id(step)?)),
    }
}

fn file_id(step: &Step) -> Result<&str, Box<HelperError>> {
    let Some(file_id) = step.file_id.as_deref() else {
        return Err(invalid("file_id"));
    };
    Ok(file_id)
}

fn invalid(field: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: format!("an azure_files step needs {field}"),
    })
}
