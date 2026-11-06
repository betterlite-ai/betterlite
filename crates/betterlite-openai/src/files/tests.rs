#![cfg(test)]

use std::ffi::OsStr;
use std::fs;
use std::future::{Future, ready};
use std::io;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use uuid::Uuid;

use super::error::{FilesError, StatusError};
use super::records::{
    CreatePurpose, ExpiresAfter, FileContentRequest, FileCreate, FileObject, FileQuery, FileUpload,
    ListKind, decode_file_object,
};
use super::stream::{
    DEFAULT_CHUNK_SIZE, FileContentStreamingResult, StreamExit, flatten_headers, read_error_body,
};
use super::transport::{FileTransport, StreamReply};
use super::{
    ClientArgs, ClientKind, DEFAULT_MAX_RETRIES, DEFAULT_TIMEOUT, FilesApi, FilesClient,
    get_openai_client, select_client,
};

const FILE_JSON: &str = r#"{"id":"file-1","bytes":3,"created_at":7,"filename":"a.jsonl","object":"file","purpose":"batch"}"#;

struct Fixture {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    chunks: Vec<Vec<u8>>,
    fail_after: bool,
    seen: Mutex<Vec<OutboundRequest>>,
}

impl Fixture {
    fn answering(status: u16, body: &str) -> Arc<Self> {
        Arc::new(Self {
            status,
            headers: Vec::new(),
            chunks: vec![body.as_bytes().to_vec()],
            fail_after: false,
            seen: Mutex::new(Vec::new()),
        })
    }

    fn headers(&self) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in &self.headers {
            map.append(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        map
    }

    fn buffered_reply(&self) -> UpstreamResponse {
        UpstreamResponse {
            status: self.status,
            headers: self.headers(),
            body: self.chunks.concat(),
        }
    }

    fn stream_reply(&self) -> StreamReply {
        let mut items: Vec<Result<Vec<u8>, HttpError>> =
            self.chunks.iter().cloned().map(Ok).collect();
        if self.fail_after {
            items.push(Err(HttpError::Timeout));
        }
        StreamReply {
            status: self.status,
            headers: self.headers(),
            body: stream::iter(items).boxed(),
        }
    }

    fn only_request(&self) -> OutboundRequest {
        let mut seen = self.seen.lock().expect("fixture lock");
        assert_eq!(seen.len(), 1, "exactly one request");
        seen.remove(0)
    }
}

impl FileTransport for Arc<Fixture> {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.seen.lock().expect("fixture lock").push(request);
        ready(Ok(self.buffered_reply()))
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        self.seen.lock().expect("fixture lock").push(request);
        ready(Ok(self.stream_reply()))
    }
}

fn client(key: &str, organization: Option<&str>) -> FilesClient {
    let args = ClientArgs {
        api_key: Some(SecretString::from(key.to_owned())),
        api_base: Some("https://api.test/v1".to_owned()),
        timeout: None,
        max_retries: None,
        organization: organization.map(str::to_owned),
    };
    select_client(get_openai_client(args, None, true).expect("client"), true).expect("selected")
}

fn api(fixture: &Arc<Fixture>) -> FilesApi<Arc<Fixture>> {
    FilesApi {
        transport: Arc::clone(fixture),
        boundaries: || Uuid::from_u128(0x0123_4567),
    }
}

fn content_request(file_id: &str) -> FileContentRequest {
    FileContentRequest {
        file_id: file_id.to_owned(),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

fn create_data() -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: Some("a.jsonl".to_owned()),
            content_type: Some("application/jsonl".to_owned()),
            data: b"{}".to_vec(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: Some(ExpiresAfter { seconds: 3600 }),
        extra_headers: IndexMap::from([("x-trace".to_owned(), "t1".to_owned())]),
        extra_body: IndexMap::from([("k".to_owned(), "v".to_owned())]),
        timeout: Some(Duration::from_secs(9)),
    }
}

fn header(request: &OutboundRequest, name: &str) -> String {
    let value = request.headers.get(name).expect("header present");
    value.to_str().expect("visible ascii header").to_owned()
}

#[derive(Clone, Copy)]
enum Scenario {
    Upload,
    ListPurpose,
    RetrieveEscapedId,
    DeleteObject,
    BinaryContent,
    StreamCancel,
    ClientProjection,
    ClientMissing,
    ClientWrongAsync,
    ErrorPassthrough,
}

#[rstest::rstest]
#[case::upload_purpose(Scenario::Upload)]
#[case::list_purpose_pagination(Scenario::ListPurpose)]
#[case::retrieve_escaped_id(Scenario::RetrieveEscapedId)]
#[case::delete_object(Scenario::DeleteObject)]
#[case::binary_file_content(Scenario::BinaryContent)]
#[case::stream_content_cancel(Scenario::StreamCancel)]
#[case::sdk_client_argument_projection(Scenario::ClientProjection)]
#[case::sdk_client_missing(Scenario::ClientMissing)]
#[case::sdk_client_wrong_async(Scenario::ClientWrongAsync)]
#[case::sdk_error_passthrough(Scenario::ErrorPassthrough)]
#[tokio::test]
async fn files_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Upload => upload().await,
        Scenario::ListPurpose => list_purpose().await,
        Scenario::RetrieveEscapedId => retrieve_escaped_id().await,
        Scenario::DeleteObject => delete_object().await,
        Scenario::BinaryContent => binary_content().await,
        Scenario::StreamCancel => stream_cancel().await,
        Scenario::ClientProjection => client_projection(),
        Scenario::ClientMissing => client_missing(),
        Scenario::ClientWrongAsync => client_wrong_async(),
        Scenario::ErrorPassthrough => error_passthrough().await,
    }
}

async fn upload() {
    let fixture = Fixture::answering(200, FILE_JSON);
    let object = api(&fixture)
        .create_file(&client("sk-a", Some("org-a")), &create_data())
        .await
        .expect("created");
    assert_eq!(object.id, "file-1");
    assert_eq!(object.bytes, 3);
    let request = fixture.only_request();
    assert_eq!(request.method, http::Method::POST);
    assert_eq!(request.url.as_str(), "https://api.test/v1/files");
    assert_eq!(request.deadline, Duration::from_secs(9));
    assert_eq!(header(&request, "authorization"), "Bearer sk-a");
    assert_eq!(header(&request, "openai-organization"), "org-a");
    assert_eq!(header(&request, "x-trace"), "t1");
    let RequestBody::Multipart { boundary, parts } = request.body else {
        panic!("upload is multipart");
    };
    assert_eq!(boundary, "00000000000000000000000001234567");
    assert_upload_parts(&parts);
    guessed_part_types();
}

fn guessed_part_types() {
    for (filename, guessed) in [
        (Some("a.txt"), Some("text/plain")),
        (Some("b.JSON"), Some("application/json")),
        (Some("x.csv"), Some("text/csv")),
        (Some("noext"), Some("application/octet-stream")),
        (None, Some("application/octet-stream")),
        (Some(""), None),
    ] {
        let mut data = create_data();
        data.file.filename = filename.map(str::to_owned);
        data.file.content_type = None;
        let request = client("sk-a", None)
            .create_request(&data, "b".to_owned())
            .expect("request");
        let RequestBody::Multipart { parts, .. } = request.body else {
            panic!("upload is multipart");
        };
        let file = parts.last().expect("file part");
        assert_eq!(file.content_type.as_deref(), guessed, "{filename:?}");
        assert_eq!(
            file.file_name.as_deref(),
            Some(filename.unwrap_or("upload"))
        );
    }
}

fn assert_upload_parts(parts: &[MultipartPart]) {
    let names: Vec<&str> = parts.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "purpose",
            "expires_after[anchor]",
            "expires_after[seconds]",
            "k",
            "file"
        ]
    );
    assert_eq!(parts.len(), 5);
    assert_eq!(parts[0].data, b"batch");
    assert_eq!(parts[2].data, b"3600");
    assert_eq!(parts[4].file_name.as_deref(), Some("a.jsonl"));
    assert_eq!(parts[4].content_type.as_deref(), Some("application/jsonl"));
}

async fn list_purpose() {
    for (purpose, expected) in [
        (Some("batch"), vec![("purpose", "batch")]),
        (Some(""), Vec::new()),
        (None, Vec::new()),
    ] {
        let fixture = Fixture::answering(200, r#"{"object":"list","data":[],"has_more":false}"#);
        let query = FileQuery {
            purpose: purpose.map(str::to_owned),
        };
        let page = api(&fixture)
            .list_files(&client("sk-a", None), &query)
            .await
            .expect("listed");
        assert_eq!(page.object, Some(ListKind::List));
        assert_eq!(page.has_more, Some(false));
        let request = fixture.only_request();
        assert_eq!(request.url.as_str(), "https://api.test/v1/files");
        let sent: Vec<(&str, &str)> = request
            .query
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        assert_eq!(sent, expected);
    }
}

async fn retrieve_escaped_id() {
    for (file_id, path) in [
        ("file-1", "/v1/files/file-1"),
        ("a/b?c#d e", "/v1/files/a%2Fb%3Fc%23d%20e"),
        ("a:b@c,d;e=f", "/v1/files/a:b@c,d;e=f"),
        ("a\u{e9}b", "/v1/files/a%C3%A9b"),
        ("%2e", "/v1/files/%252e"),
        ("...", "/v1/files/..."),
    ] {
        let fixture = Fixture::answering(200, FILE_JSON);
        api(&fixture)
            .retrieve_file(&client("sk-a", None), file_id)
            .await
            .expect("retrieved");
        let request = fixture.only_request();
        assert_eq!(request.method, http::Method::GET);
        assert_eq!(request.url.path(), path);
    }
    for file_id in [".", ".."] {
        let fixture = Fixture::answering(200, FILE_JSON);
        let failure = api(&fixture)
            .retrieve_file(&client("sk-a", None), file_id)
            .await
            .expect_err("dot segment");
        assert!(
            matches!(failure, FilesError::DotSegment { .. }),
            "{file_id}"
        );
        assert!(fixture.seen.lock().expect("lock").is_empty());
    }
    let empty = api(&Fixture::answering(200, FILE_JSON))
        .retrieve_file(&client("sk-a", None), "")
        .await
        .expect_err("empty id");
    assert_eq!(
        empty.to_string(),
        "Expected a non-empty value for `file_id` but received ''"
    );
}

async fn delete_object() {
    let body = r#"{"id":"file-1","object":"file","deleted":true}"#;
    let fixture = Fixture::answering(200, body);
    let deleted = api(&fixture)
        .delete_file(&client("sk-a", None), "file-1")
        .await
        .expect("deleted");
    assert_eq!(serde_json::to_string(&deleted).expect("serializable"), body);
    let request = fixture.only_request();
    assert_eq!(request.method, http::Method::DELETE);
    assert_eq!(request.url.path(), "/v1/files/file-1");
    assert!(matches!(request.body, RequestBody::Empty));
}

fn streaming_fixture(chunks: Vec<Vec<u8>>, fail_after: bool) -> Arc<Fixture> {
    Arc::new(Fixture {
        status: 200,
        headers: vec![
            ("content-type", "application/octet-stream"),
            ("x-multi", "a"),
        ],
        chunks,
        fail_after,
        seen: Mutex::new(Vec::new()),
    })
}

async fn chunk_lengths(
    source: &Arc<Fixture>,
    chunk_size: Option<NonZeroUsize>,
) -> (Vec<Vec<u8>>, Arc<OnceLock<StreamExit>>) {
    let mut opened = api(source)
        .file_content_streaming(&client("sk-a", None), &content_request("f"), chunk_size)
        .await
        .expect("streaming");
    let mut collected = Vec::new();
    while let Some(chunk) = opened.stream.next().await {
        collected.push(chunk.expect("chunk"));
    }
    (collected, opened.exit)
}

async fn binary_content() {
    let fixture = streaming_fixture(vec![vec![0, 255, 10]], false);
    let content = api(&fixture)
        .file_content(&client("sk-a", None), &content_request("f/1"))
        .await
        .expect("content");
    assert_eq!(content.body, [0, 255, 10]);
    assert_eq!(content.status, 200);
    assert_eq!(content.headers["content-type"], "application/octet-stream");
    let request = fixture.only_request();
    assert_eq!(request.url.path(), "/v1/files/f%2F1/content");
    assert_eq!(header(&request, "accept"), "application/binary");
    binary_rechunking().await;
}

async fn binary_rechunking() {
    let size = DEFAULT_CHUNK_SIZE;
    let large = streaming_fixture(vec![vec![1; size + 3], vec![2; size + 2]], false);
    let (large_chunks, _) = chunk_lengths(&large, None).await;
    let lengths: Vec<usize> = large_chunks.iter().map(Vec::len).collect();
    assert_eq!(size, 0x0010_0000);
    assert_eq!(lengths, [size, size, 5]);

    let uneven = streaming_fixture(
        vec![b"abc".to_vec(), Vec::new(), b"defghij".to_vec()],
        false,
    );
    let (uneven_chunks, _) = chunk_lengths(&uneven, NonZeroUsize::new(4)).await;
    assert_eq!(
        uneven_chunks,
        [b"abcd".to_vec(), b"efgh".to_vec(), b"ij".to_vec()]
    );

    let mut headers = HeaderMap::new();
    headers.append("Set-Cookie", HeaderValue::from_static("a=1"));
    headers.append("set-cookie", HeaderValue::from_static("b=2"));
    assert_eq!(flatten_headers(&headers)["set-cookie"], "a=1, b=2");
}

async fn open_stream(
    source: &Arc<Fixture>,
    chunk_size: Option<NonZeroUsize>,
) -> FileContentStreamingResult {
    api(source)
        .file_content_streaming(&client("sk-a", None), &content_request("f"), chunk_size)
        .await
        .expect("streaming")
}

async fn stream_cancel() {
    let fixture = streaming_fixture(vec![b"abcdef".to_vec()], false);
    let (finished, finished_exit) = chunk_lengths(&fixture, NonZeroUsize::new(4)).await;
    assert_eq!(finished.len(), 2);
    assert_eq!(finished_exit.get(), Some(&StreamExit::Completed));

    let opened = open_stream(&fixture, NonZeroUsize::new(2)).await;
    assert_eq!(opened.headers["x-multi"], "a");
    assert_eq!(
        fixture.seen.lock().expect("lock").len(),
        2,
        "entered before any read"
    );
    assert_eq!(opened.exit.get(), None);
    let (mut stream, closed_exit) = (opened.stream, opened.exit);
    stream.next().await.expect("first").expect("chunk");
    assert_eq!(closed_exit.get(), None);
    drop(stream);
    assert_eq!(closed_exit.get(), Some(&StreamExit::Closed));

    stream_fails().await;

    // Python never exits the context of a generator that was never started; betterlite always does
    let unread = open_stream(&fixture, None).await;
    let unread_exit = Arc::clone(&unread.exit);
    drop(unread);
    assert_eq!(unread_exit.get(), Some(&StreamExit::Closed));
}

async fn stream_fails() {
    let failing = streaming_fixture(vec![b"abc".to_vec()], true);
    let mut opened = open_stream(&failing, NonZeroUsize::new(2)).await;
    opened.stream.next().await.expect("first").expect("chunk");
    let broken = opened
        .stream
        .next()
        .await
        .expect("error item")
        .expect_err("broken");
    assert!(matches!(broken, FilesError::Body { .. }));
    assert!(matches!(opened.exit.get(), Some(StreamExit::Failed(_))));
    assert!(opened.stream.next().await.is_none());
}

fn client_projection() {
    let full = ClientArgs {
        api_key: Some(SecretString::from(String::new())),
        api_base: Some("https://x.test/v1".to_owned()),
        timeout: Some(Duration::ZERO),
        max_retries: Some(0),
        organization: Some(String::new()),
    };
    let Some(ClientKind::Async(built)) = get_openai_client(full, None, true).expect("built") else {
        panic!("async client expected");
    };
    assert_eq!(built.base_url.as_str(), "https://x.test/v1/");
    assert_eq!(built.timeout, Duration::ZERO);
    assert_eq!(built.max_retries, 0);
    assert_eq!(built.organization.as_deref(), Some(""));

    let bare = ClientArgs {
        api_key: Some(SecretString::from("k".to_owned())),
        api_base: None,
        timeout: None,
        max_retries: None,
        organization: None,
    };
    let Some(ClientKind::Sync(defaults)) = get_openai_client(bare, None, false).expect("built")
    else {
        panic!("sync client expected");
    };
    assert_eq!(defaults.base_url.as_str(), "https://api.openai.com/v1/");
    assert_eq!(defaults.timeout, DEFAULT_TIMEOUT);
    assert_eq!(defaults.max_retries, DEFAULT_MAX_RETRIES);
    assert_eq!(defaults.organization, None);
    supplied_client_wins();
}

fn supplied_client_wins() {
    let supplied = ClientKind::Sync(client("sk-supplied", None));
    let ignored = ClientArgs {
        api_key: Some(SecretString::from("ignored".to_owned())),
        api_base: Some("https://other.test".to_owned()),
        timeout: None,
        max_retries: None,
        organization: None,
    };
    let retained = get_openai_client(ignored, Some(supplied), true).expect("kept");
    assert!(
        matches!(retained, Some(ClientKind::Sync(held)) if held.base_url.host_str() == Some("api.test"))
    );
    assert!(
        get_openai_client(ClientArgs::default(), None, true)
            .expect("none")
            .is_none()
    );
}

fn client_missing() {
    for is_async in [true, false] {
        let failure = select_client(None, is_async).expect_err("no client");
        assert!(matches!(failure, FilesError::NotInitialized));
        assert_eq!(
            failure.to_string(),
            "OpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment."
        );
    }
}

fn client_wrong_async() {
    let sync = || Some(ClientKind::Sync(client("sk-a", None)));
    let failure = select_client(sync(), true).expect_err("sync client for async");
    assert_eq!(
        failure.to_string(),
        "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client."
    );
    select_client(sync(), false).expect("the sync paths only cast");
    select_client(Some(ClientKind::Async(client("sk-a", None))), false)
        .expect("async client used synchronously");
}

async fn error_passthrough() {
    let body = r#"{"error":{"message":"No such File object: file-x","type":"invalid_request_error","param":"id","code":"file_not_found"}}"#;
    let fixture = Fixture::answering(404, body);
    let files = api(&fixture);
    let sdk = client("sk-a", None);
    let query = FileQuery::default();
    let failures = [
        files.create_file(&sdk, &create_data()).await.map(drop),
        files
            .file_content(&sdk, &content_request("file-x"))
            .await
            .map(drop),
        files
            .file_content_streaming(&sdk, &content_request("file-x"), None)
            .await
            .map(drop),
        files.retrieve_file(&sdk, "file-x").await.map(drop),
        files.delete_file(&sdk, "file-x").await.map(drop),
        files.list_files(&sdk, &query).await.map(drop),
    ];
    for failure in failures {
        let FilesError::Status(status) = failure.expect_err("status error") else {
            panic!("a status error passes through");
        };
        assert_eq!(status.status, 404);
        assert_eq!(status.error_type.as_deref(), Some("invalid_request_error"));
        assert_eq!(status.param.as_deref(), Some("id"));
        assert_eq!(status.code.as_deref(), Some("file_not_found"));
        assert!(status.message.starts_with("Error code: 404 - "));
        assert!(status.message.contains("No such File object: file-x"));
    }
    status_error_parsing();
    error_body_limit().await;
}

// Reading stops once 64 KiB are collected, so 65 chunks of 1 KiB keep 64 of them
async fn error_body_limit() {
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        vec![vec![b'x'; 1024]; 65].into_iter().map(Ok).collect();
    let collected = read_error_body(stream::iter(chunks).boxed()).await;
    assert_eq!(collected.len(), 64 * 1024);
}

fn status_error_parsing() {
    assert_eq!(
        StatusError::from_response(502, b" upstream down \n"),
        StatusError {
            status: 502,
            message: "upstream down".to_owned(),
            error_type: None,
            param: None,
            code: None
        }
    );
    assert_eq!(
        StatusError::from_response(500, b"").message,
        "Error code: 500"
    );
    let flat = StatusError::from_response(400, br#"{"type":"t","code":"c","message":"m"}"#);
    assert_eq!(flat.error_type.as_deref(), Some("t"));
    assert_eq!(flat.code.as_deref(), Some("c"));
}

#[tokio::test]
async fn openai_files_credential_identity_isolation() {
    let first = Fixture::answering(200, FILE_JSON);
    let mut marked = create_data();
    marked
        .extra_headers
        .insert("x-only-first".to_owned(), "1".to_owned());
    api(&first)
        .create_file(&client("sk-first", Some("org-first")), &marked)
        .await
        .expect("created");
    let second = Fixture::answering(200, FILE_JSON);
    api(&second)
        .create_file(&client("sk-second", Some("org-second")), &create_data())
        .await
        .expect("created");
    let again = Fixture::answering(200, FILE_JSON);
    api(&again)
        .create_file(&client("sk-first", Some("org-first")), &create_data())
        .await
        .expect("created");
    let seen: Vec<(String, String, bool)> = [first, second, again]
        .iter()
        .map(|fixture| {
            let request = fixture.only_request();
            (
                header(&request, "authorization"),
                header(&request, "openai-organization"),
                request.headers.contains_key("x-only-first"),
            )
        })
        .collect();
    let expected = [
        ("Bearer sk-first", "org-first", true),
        ("Bearer sk-second", "org-second", false),
        ("Bearer sk-first", "org-first", false),
    ]
    .map(|(auth, org, marker)| (auth.to_owned(), org.to_owned(), marker));
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn openai_files_secret_safe_failure() {
    let secret = "sk-fixture-credential";
    let rendered = format!("{:?}", client(secret, Some("org-x")));
    assert!(!rendered.contains(secret), "{rendered}");

    let broken = client(&format!("{secret}\n"), None);
    let bad_header = broken
        .create_request(&create_data(), "b".to_owned())
        .expect_err("bad header");
    let header_text = format!("{bad_header} {bad_header:?}");
    assert!(matches!(bad_header, FilesError::Header { .. }));
    assert!(!header_text.contains(secret), "{header_text}");

    let fixture = Fixture::answering(
        401,
        r#"{"error":{"message":"Incorrect API key provided: sk-fix****"}}"#,
    );
    let rejection = api(&fixture)
        .retrieve_file(&client(secret, None), "file-1")
        .await
        .expect_err("rejected");
    let rejection_text = format!("{rejection} {rejection:?}");
    assert!(!rejection_text.contains(secret), "{rejection_text}");
    let request = fixture.only_request();
    assert!(!format!("{request:?}").contains(secret));
}

#[test]
fn replays_the_openai_files_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_files");
    fs::read_dir(corpus)
        .expect("corpus folder")
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .expect("utf-8 file name");
            let decoded = decode_file_object::<FileObject>(200, &bytes);
            if name.starts_with("valid") || name.starts_with("boundary_ok") {
                decoded.expect("this seed is a file object");
            } else {
                assert!(matches!(decoded, Err(FilesError::Decode { .. })), "{name}");
            }
            Ok::<(), io::Error>(())
        })
        .expect("readable corpus");
}
