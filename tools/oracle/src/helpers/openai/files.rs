use std::num::NonZeroUsize;
use std::time::Duration;

use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{
    CreatePurpose, ExpiresAfter, FileContentRequest, FileCreate, FileQuery, FileUpload,
};
use betterlite_openai::files::stream::StreamExit;
use betterlite_openai::files::{
    ClientArgs, ClientKind, FilesApi, FilesClient, get_openai_client, select_client,
};
use fixture::{Fixture, Reply};
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

mod fixture;
mod tests;

pub(crate) struct Op;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    CreateFile,
    FileContent,
    FileContentStreaming,
    RetrieveFile,
    DeleteFile,
    ListFiles,
    Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Supplied {
    Sync,
    Async,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientSpec {
    api_key: Option<String>,
    api_base: Option<String>,
    timeout: Option<u64>,
    max_retries: Option<u32>,
    organization: Option<String>,
    supplied: Option<Supplied>,
    is_async: bool,
    #[serde(default)]
    missing: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(default)]
    file_id: String,
    #[serde(default)]
    purpose: Option<CreatePurpose>,
    #[serde(default)]
    list_purpose: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    data: String,
    #[serde(default)]
    expires_after: Option<i64>,
    #[serde(default)]
    extra_headers: IndexMap<String, String>,
    #[serde(default)]
    extra_body: IndexMap<String, String>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    chunk_size: Option<NonZeroUsize>,
    #[serde(default)]
    read: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    action: Action,
    client: ClientSpec,
    #[serde(default)]
    request: Request,
    #[serde(default)]
    reply: Option<Reply>,
}

fn client_args(spec: &ClientSpec) -> ClientArgs {
    ClientArgs {
        api_key: spec.api_key.clone().map(SecretString::from),
        api_base: spec.api_base.clone(),
        timeout: spec.timeout.map(Duration::from_secs),
        max_retries: spec.max_retries,
        organization: spec.organization.clone(),
    }
}

const fn kind_name(kind: &ClientKind) -> &'static str {
    match kind {
        ClientKind::Async(_) => "async",
        ClientKind::Sync(_) => "sync",
    }
}

fn client_view(spec: &ClientSpec) -> Result<Value, FilesError> {
    let held = match spec.supplied {
        Some(kind) => {
            let args = ClientArgs {
                api_key: Some(SecretString::from("sk-supplied".to_owned())),
                api_base: Some("https://supplied.test/v1".to_owned()),
                timeout: None,
                max_retries: None,
                organization: None,
            };
            get_openai_client(args, None, kind == Supplied::Async)?
        }
        None => None,
    };
    let built = get_openai_client(client_args(spec), held, spec.is_async)?;
    let Some(kind) = built else {
        return Ok(Value::Null);
    };
    let (ClientKind::Async(client) | ClientKind::Sync(client)) = &kind;
    Ok(json!({
        "kind": kind_name(&kind),
        "base_url": client.base_url.as_str(),
        "timeout": client.timeout.as_secs(),
        "max_retries": client.max_retries,
        "organization": client.organization,
        "key_is_given": spec.api_key.as_deref() == Some(client.api_key.expose_secret()),
    }))
}

// The client the lifecycle call receives: built from the spec, or absent as a patched get_openai_client leaves it
fn action_client(spec: &ClientSpec) -> Result<FilesClient, FilesError> {
    let built = if spec.missing {
        None
    } else {
        get_openai_client(
            client_args(spec),
            None,
            spec.supplied
                .map_or(spec.is_async, |kind| kind == Supplied::Async),
        )?
    };
    select_client(built, spec.is_async)
}

fn create_data(request: &Request) -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: request.filename.clone(),
            content_type: request.content_type.clone(),
            data: request.data.clone().into_bytes(),
        },
        purpose: request.purpose.unwrap_or(CreatePurpose::Batch),
        expires_after: request
            .expires_after
            .map(|seconds| ExpiresAfter { seconds }),
        extra_headers: request.extra_headers.clone(),
        extra_body: request.extra_body.clone(),
        timeout: request.timeout.map(Duration::from_secs),
    }
}

fn content_request(request: &Request) -> FileContentRequest {
    FileContentRequest {
        file_id: request.file_id.clone(),
        extra_headers: request.extra_headers.clone(),
        extra_body: request.extra_body.clone(),
        timeout: request.timeout.map(Duration::from_secs),
    }
}

async fn read_stream(
    api: &FilesApi<&Fixture>,
    client: &FilesClient,
    input: &Input,
    fixture: &Fixture,
) -> Result<Value, FilesError> {
    let opened = api
        .file_content_streaming(
            client,
            &content_request(&input.request),
            input.request.chunk_size,
        )
        .await?;
    let mut stream = opened.stream;
    let mut chunks = Vec::new();
    let mut broke = false;
    while input.request.read.is_none_or(|limit| chunks.len() < limit) {
        match stream.next().await {
            Some(Ok(chunk)) => chunks.push(chunk),
            Some(Err(_)) => {
                broke = true;
                break;
            }
            None => break,
        }
    }
    drop(stream);
    let exit = match opened.exit.get() {
        Some(StreamExit::Completed) => "completed",
        Some(StreamExit::Failed(_)) => "failed",
        Some(StreamExit::Closed) | None => "closed",
    };
    Ok(json!({
        "headers": fixture.watched(&opened.headers),
        "chunks": chunks,
        "broke": broke,
        "exit": exit,
    }))
}

async fn invoke(
    client: &FilesClient,
    input: &Input,
    fixture: &Fixture,
) -> Result<Value, FilesError> {
    let api = FilesApi::new(fixture, || Uuid::from_u128(0x0123_4567));
    let request = &input.request;
    match input.action {
        Action::CreateFile => Ok(json!(api.create_file(client, &create_data(request)).await?)),
        Action::FileContent => {
            let content = api.file_content(client, &content_request(request)).await?;
            Ok(json!({
                "status": content.status,
                "headers": fixture.watched(&content.headers),
                "body": content.body,
            }))
        }
        Action::FileContentStreaming => read_stream(&api, client, input, fixture).await,
        Action::RetrieveFile => api.retrieve_file(client, &request.file_id).await,
        Action::DeleteFile => api.delete_file(client, &request.file_id).await,
        Action::ListFiles => {
            let query = FileQuery {
                purpose: request.list_purpose.clone(),
            };
            api.list_files(client, &query).await
        }
        Action::Client => Ok(Value::Null),
    }
}

// SDK status errors keep their fields; every other failure the SDK raises is a ValueError
fn failure_view(error: &FilesError) -> Value {
    if let FilesError::Status(status) = error {
        return json!({"raised": {
            "kind": "status",
            "status": status.status,
            "type": status.error_type,
            "param": status.param,
            "code": status.code,
            "message": status.message,
        }});
    }
    json!({"raised": {"kind": "ValueError", "message": error.to_string()}})
}

fn failure<M>(message: M) -> Box<crate::helpers::HelperError>
where
    M: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "OpenAiFilesError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        if matches!(input.action, Action::Client) {
            return Ok(client_view(&input.client).unwrap_or_else(|error| failure_view(&error)));
        }
        let fixture = Fixture::new(input.reply.as_ref()).map_err(failure)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(failure)?;
        let outcome = runtime.block_on(async {
            let client = action_client(&input.client)?;
            invoke(&client, input, &fixture).await
        });
        Ok(json!({
            "request": fixture.request_view(&input.request.extra_headers),
            "result": outcome.unwrap_or_else(|error| failure_view(&error)),
        }))
    }
}
