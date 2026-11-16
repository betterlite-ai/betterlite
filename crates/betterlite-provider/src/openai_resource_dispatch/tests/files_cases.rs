use std::num::NonZeroUsize;
use std::time::Duration;

use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{
    CreatePurpose, ExpiresAfter, FileContentRequest, FileCreate, FileQuery, FileUpload,
};
use betterlite_openai::files::stream::StreamExit;
use betterlite_openai::files::{FilesApi, FilesClient};
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use uuid::Uuid;

use super::fixture::{Fixture, WireBody, files_client};
use crate::openai_resource_dispatch::files::{Call, Dispatch, Outcome};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

fn api(fixture: Fixture) -> FilesApi<Fixture> {
    FilesApi {
        transport: fixture,
        boundaries: Uuid::nil,
    }
}

async fn run(
    api: &FilesApi<Fixture>,
    client: &FilesClient,
    call: Call<'_>,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let operation = OpenAiResourceOperation::files(Dispatch { api, client, call });
    dispatch(operation).await
}

fn file_object() -> Vec<u8> {
    br#"{"id":"file-1","bytes":8,"created_at":1700000000,"filename":"batch.jsonl","object":"file","purpose":"batch","status":"processed"}"#.to_vec()
}

fn page_body() -> Vec<u8> {
    br#"{"object":"list","data":[{"id":"file-1","bytes":8,"created_at":1700000000,"filename":"batch.jsonl","object":"file","purpose":"batch"}],"has_more":false,"first_id":"file-1","last_id":"file-1"}"#.to_vec()
}

fn create_data() -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: Some(String::from("batch.jsonl")),
            content_type: Some(String::from("application/jsonl")),
            data: b"{\"x\":1}\n".to_vec(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: Some(ExpiresAfter { seconds: 3600 }),
        extra_headers: IndexMap::from([(String::from("x-trace"), String::from("t1"))]),
        extra_body: IndexMap::from([(String::from("note"), String::from("n"))]),
        timeout: Some(Duration::from_secs(30)),
    }
}

async fn create_row() {
    let fixture = Fixture::replied(200, &file_object());
    let api = api(fixture);
    let client = files_client();
    let data = create_data();
    let result = run(&api, &client, Call::Create(&data)).await;
    let Ok(OpenAiResourceResult::Files(Outcome::Object(file))) = result else {
        panic!("the create reply decodes to a file object");
    };
    assert_eq!(file.id, "file-1");
    let seen = api.transport.seen();
    let wire = &seen[0];
    assert_eq!(wire.method, "POST");
    assert_eq!(wire.path, "/v1/files");
    assert_eq!(wire.query, Vec::new());
    assert_eq!(wire.header("authorization"), Some("Bearer sk-files"));
    assert_eq!(wire.header("openai-organization"), Some("org-files"));
    assert_eq!(wire.header("accept"), Some("application/json"));
    assert_eq!(wire.header("x-trace"), Some("t1"));
    let WireBody::Multipart(parts) = &wire.body else {
        panic!("a file upload is multipart");
    };
    let names: Vec<&str> = parts.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "purpose",
            "expires_after[anchor]",
            "expires_after[seconds]",
            "note",
            "file"
        ]
    );
    assert_eq!(parts[0].data, b"batch".to_vec());
    assert_eq!(parts[3].data, b"n".to_vec());
    assert_eq!(parts[4].file_name.as_deref(), Some("batch.jsonl"));
    assert_eq!(parts[4].content_type.as_deref(), Some("application/jsonl"));
    assert_eq!(parts[4].data, b"{\"x\":1}\n".to_vec());
}

async fn retrieve_row() {
    let fixture = Fixture::replied(200, &file_object());
    let api = api(fixture);
    let client = files_client();
    let result = run(&api, &client, Call::Retrieve("file-1")).await;
    let Ok(OpenAiResourceResult::Files(Outcome::Object(file))) = result else {
        panic!("the retrieve reply decodes to a file object");
    };
    assert_eq!(file.filename, "batch.jsonl");
    let seen = api.transport.seen();
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/v1/files/file-1");
    assert_eq!(seen[0].body, WireBody::Empty);
}

async fn delete_row() {
    let fixture = Fixture::replied(200, br#"{"id":"file-1","object":"file","deleted":true}"#);
    let api = api(fixture);
    let client = files_client();
    let result = run(&api, &client, Call::Delete("file-1")).await;
    let Ok(OpenAiResourceResult::Files(Outcome::Deleted(deleted))) = result else {
        panic!("the delete reply decodes to a deleted file");
    };
    assert!(deleted.deleted);
    let seen = api.transport.seen();
    assert_eq!(seen[0].method, "DELETE");
    assert_eq!(seen[0].path, "/v1/files/file-1");
}

async fn list_row() {
    let fixture = Fixture::replied(200, &page_body());
    let api = api(fixture);
    let client = files_client();
    let query = FileQuery {
        purpose: Some(String::from("batch")),
    };
    let result = run(&api, &client, Call::List(&query)).await;
    let Ok(OpenAiResourceResult::Files(Outcome::Page(page))) = result else {
        panic!("the list reply decodes to a page");
    };
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.first_id.as_deref(), Some("file-1"));
    let seen = api.transport.seen();
    assert_eq!(
        seen[0].query,
        vec![(String::from("purpose"), String::from("batch"))]
    );
}

async fn content_row() {
    let fixture = Fixture::replied(200, b"line one\nline two\n");
    let api = api(fixture);
    let client = files_client();
    let request = FileContentRequest {
        file_id: String::from("file-1"),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    let result = run(&api, &client, Call::Content(&request)).await;
    let Ok(OpenAiResourceResult::Files(Outcome::Content(content))) = result else {
        panic!("the content reply decodes to bytes");
    };
    assert_eq!(content.status, 200);
    assert_eq!(content.body, b"line one\nline two\n".to_vec());
    let seen = api.transport.seen();
    assert_eq!(seen[0].path, "/v1/files/file-1/content");
    assert_eq!(seen[0].header("accept"), Some("application/binary"));
}

async fn streaming_row() {
    let fixture = Fixture::streamed(vec![b"ab".to_vec(), b"cdef".to_vec()]);
    let api = api(fixture);
    let client = files_client();
    let request = FileContentRequest {
        file_id: String::from("file-1"),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    let chunk_size = NonZeroUsize::new(2);
    let result = run(
        &api,
        &client,
        Call::StreamingContent {
            request: &request,
            chunk_size,
        },
    )
    .await;
    let Ok(OpenAiResourceResult::Files(Outcome::Streaming(mut stream))) = result else {
        panic!("the streaming reply stays a stream");
    };
    assert_eq!(
        stream.headers.get("x-request-id").map(String::as_str),
        Some("req-stream")
    );
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    while let Some(chunk) = stream.stream.next().await {
        chunks.push(chunk.expect("the recorded chunk reads back"));
    }
    assert_eq!(chunks, vec![b"ab".to_vec(), b"cd".to_vec(), b"ef".to_vec()]);
    assert_eq!(stream.exit.get(), Some(&StreamExit::Completed));
}

async fn error_row() {
    let body = br#"{"error":{"message":"bad file","type":"invalid_request_error","param":"file_id","code":"invalid"}}"#;
    let fixture = Fixture::replied(400, body);
    let api = api(fixture);
    let client = files_client();
    let result = run(&api, &client, Call::Retrieve("file-1")).await;
    let Err(error) = result else {
        panic!("a 400 reply is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Files(FilesError::Status(_))
    ));
}

pub(crate) async fn check() {
    create_row().await;
    retrieve_row().await;
    delete_row().await;
    list_row().await;
    content_row().await;
    streaming_row().await;
    error_row().await;
}
