#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use super::{
    DOWNLOAD_TIMEOUT, GigachatFileTransport, GigachatFileUploader, GigachatFilesConfig,
    UPLOAD_TIMEOUT,
};
use crate::gigachat_chat::config::{GigachatBearer, GigachatImageUploader as _};
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::IdSource;

const DEFAULT_API_BASE: &str = "https://api.test/v1";
const DATA_URL: &str = "data:image/png;base64,QUJD";
const FIXED: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;
const FIXED_SIMPLE: &str = "0123456789ab4def8123456789abcdef";
const FIXED_UUID: &str = "01234567-89ab-4def-8123-456789abcdef";
const IMAGE_URL: &str = "https://img.test/a.png";

struct FixedIds;

impl IdSource for FixedIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128(FIXED)
    }
}

struct ScriptedBearer {
    token: Mutex<Option<&'static str>>,
    status_code: u16,
    seen: Mutex<Vec<Option<String>>>,
}

impl ScriptedBearer {
    fn issuing(token: &'static str) -> Self {
        Self {
            token: Mutex::new(Some(token)),
            status_code: 200,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn failing(status_code: u16) -> Self {
        Self {
            token: Mutex::new(None),
            status_code,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn revoke(&self) {
        *self.token.lock().unwrap() = None;
    }

    fn seen(&self) -> Vec<Option<String>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl GigachatBearer for ScriptedBearer {
    async fn access_token(
        &self,
        credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigaChatError> {
        self.seen
            .lock()
            .unwrap()
            .push(credentials.map(|value| value.expose_secret().to_owned()));
        let token = *self.token.lock().unwrap();
        token.map_or_else(
            || {
                Err(GigaChatError {
                    status_code: self.status_code,
                    message: String::from("no token"),
                    headers: None,
                    source: None,
                })
            },
            |issued| Ok(SecretString::from(issued)),
        )
    }
}

enum Scripted {
    Answer(UpstreamResponse),
    Fail,
}

impl Scripted {
    fn into_result(self) -> Result<UpstreamResponse, TransportError> {
        match self {
            Self::Answer(response) => Ok(response),
            Self::Fail => Err(TransportError::BodyTooLarge { limit: 0 }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenDownload {
    method: String,
    url: String,
    deadline: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenPart {
    name: String,
    file_name: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenUpload {
    method: String,
    url: String,
    deadline: Duration,
    authorization: Option<String>,
    boundary: String,
    parts: Vec<SeenPart>,
}

#[derive(Default)]
struct FakeState {
    downloads: Mutex<Vec<SeenDownload>>,
    uploads: Mutex<Vec<SeenUpload>>,
    download_replies: Mutex<VecDeque<Scripted>>,
    upload_replies: Mutex<VecDeque<Scripted>>,
}

struct Fake {
    state: Arc<FakeState>,
}

fn seen_part(part: &MultipartPart) -> SeenPart {
    SeenPart {
        name: part.name.clone(),
        file_name: part.file_name.clone(),
        content_type: part.content_type.clone(),
        data: part.data.clone(),
    }
}

fn seen_upload(request: &OutboundRequest) -> SeenUpload {
    let (boundary, parts) = match &request.body {
        RequestBody::Multipart { boundary, parts } => {
            (boundary.clone(), parts.iter().map(seen_part).collect())
        }
        RequestBody::Empty | RequestBody::Json(_) | RequestBody::Bytes { .. } => {
            (String::new(), Vec::new())
        }
    };
    SeenUpload {
        method: request.method.as_str().to_owned(),
        url: request.url.to_string(),
        deadline: request.deadline,
        authorization: request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        boundary,
        parts,
    }
}

fn seen_download(request: &OutboundRequest) -> SeenDownload {
    SeenDownload {
        method: request.method.as_str().to_owned(),
        url: request.url.to_string(),
        deadline: request.deadline,
    }
}

fn record<T>(queue: &Mutex<Vec<T>>, entry: T) {
    queue.lock().unwrap().push(entry);
}

fn scripted(replies: &Mutex<VecDeque<Scripted>>) -> Result<UpstreamResponse, TransportError> {
    let mut queue = replies.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(reply) = queue.pop_front() else {
        return Err(TransportError::BodyTooLarge { limit: 0 });
    };
    reply.into_result()
}

impl GigachatFileTransport for Fake {
    fn download(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        record(&self.state.downloads, seen_download(&request));
        ready(scripted(&self.state.download_replies))
    }

    fn upload(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        record(&self.state.uploads, seen_upload(&request));
        ready(scripted(&self.state.upload_replies))
    }
}

#[derive(Clone, Copy)]
struct Setup {
    capacity: usize,
    api_base: Option<&'static str>,
    env_base: Option<&'static str>,
    credentials: Option<&'static str>,
    token: Option<&'static str>,
}

fn setup() -> Setup {
    Setup {
        capacity: 4,
        api_base: Some(DEFAULT_API_BASE),
        env_base: None,
        credentials: None,
        token: Some("token-1"),
    }
}

struct Fixture {
    state: Arc<FakeState>,
    bearer: Arc<ScriptedBearer>,
    uploader: GigachatFileUploader<Fake>,
}

fn fixture(given: Setup) -> Fixture {
    let state = Arc::new(FakeState::default());
    let bearer = Arc::new(
        given
            .token
            .map_or_else(|| ScriptedBearer::failing(401), ScriptedBearer::issuing),
    );
    let shared = Arc::clone(&bearer);
    let config = GigachatFilesConfig {
        bearer: shared,
        ids: Arc::new(FixedIds),
        credentials: given.credentials.map(SecretString::from),
        api_base: given.api_base.map(str::to_owned),
        api_base_env: given.env_base.map(str::to_owned),
    };
    let uploader = GigachatFileUploader::new(
        Fake {
            state: Arc::clone(&state),
        },
        config,
        given.capacity,
    );
    Fixture {
        state,
        bearer,
        uploader,
    }
}

impl Fixture {
    fn downloads(&self) -> Vec<SeenDownload> {
        self.state.downloads.lock().unwrap().clone()
    }

    fn uploads(&self) -> Vec<SeenUpload> {
        self.state.uploads.lock().unwrap().clone()
    }

    fn script_download(&self, reply: Scripted) {
        self.state.download_replies.lock().unwrap().push_back(reply);
    }

    fn script_upload(&self, reply: Scripted) {
        self.state.upload_replies.lock().unwrap().push_back(reply);
    }
}

fn answer(status: u16, body: &str) -> Scripted {
    Scripted::Answer(UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    })
}

fn answer_with_type(status: u16, content_type: &'static str, body: &[u8]) -> Scripted {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static(content_type),
    );
    Scripted::Answer(UpstreamResponse {
        status,
        headers,
        body: body.to_vec(),
    })
}

fn id_reply(id: &str) -> Scripted {
    answer(200, &format!(r#"{{"id":"{id}"}}"#))
}

fn first<T>(items: &[T]) -> &T {
    items.first().expect("one item")
}

#[tokio::test]
async fn a_data_url_is_decoded_without_a_download() {
    let fixture = fixture(setup());
    fixture.script_upload(id_reply("file-1"));
    let id = fixture.uploader.upload(DATA_URL).await;
    assert_eq!(id, Some(String::from("file-1")));
    assert!(fixture.downloads().is_empty());
    let uploads = fixture.uploads();
    assert_eq!(uploads.len(), 1);
    let upload = first(&uploads);
    assert_eq!(upload.method, "POST");
    assert_eq!(upload.url, "https://api.test/v1/files");
    assert_eq!(upload.deadline, UPLOAD_TIMEOUT);
    assert_eq!(upload.authorization.as_deref(), Some("Bearer token-1"));
    assert_eq!(upload.boundary, FIXED_SIMPLE);
    assert_eq!(
        upload.parts,
        vec![
            SeenPart {
                name: String::from("purpose"),
                file_name: None,
                content_type: None,
                data: b"general".to_vec(),
            },
            SeenPart {
                name: String::from("file"),
                file_name: Some(format!("{FIXED_UUID}.png")),
                content_type: Some(String::from("image/png")),
                data: b"ABC".to_vec(),
            },
        ]
    );
}

#[tokio::test]
async fn a_remote_image_is_downloaded_and_uploaded() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "image/gif", &[1, 2, 3]));
    fixture.script_upload(id_reply("file-2"));
    let id = fixture.uploader.upload(IMAGE_URL).await;
    assert_eq!(id, Some(String::from("file-2")));
    let downloads = fixture.downloads();
    assert_eq!(downloads.len(), 1);
    let download = first(&downloads);
    assert_eq!(download.method, "GET");
    assert_eq!(download.url, IMAGE_URL);
    assert_eq!(download.deadline, DOWNLOAD_TIMEOUT);
    let uploads = fixture.uploads();
    let part = &first(&uploads).parts[1];
    assert_eq!(part.content_type.as_deref(), Some("image/gif"));
    assert_eq!(
        part.file_name.as_deref(),
        Some("01234567-89ab-4def-8123-456789abcdef.gif")
    );
    assert_eq!(part.data, vec![1, 2, 3]);
}

#[tokio::test]
async fn a_download_without_a_content_type_uses_image_jpeg() {
    let fixture = fixture(setup());
    fixture.script_download(answer(200, "raw"));
    fixture.script_upload(id_reply("file-3"));
    assert_eq!(
        fixture.uploader.upload(IMAGE_URL).await,
        Some(String::from("file-3"))
    );
    let uploads = fixture.uploads();
    let part = &first(&uploads).parts[1];
    assert_eq!(part.content_type.as_deref(), Some("image/jpeg"));
    assert_eq!(
        part.file_name.as_deref(),
        Some("01234567-89ab-4def-8123-456789abcdef.jpg")
    );
}

#[tokio::test]
async fn a_charset_in_the_content_type_leaves_the_subtype() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "image/png; charset=binary", b"x"));
    fixture.script_upload(id_reply("file-4"));
    fixture.uploader.upload(IMAGE_URL).await;
    let uploads = fixture.uploads();
    let part = &first(&uploads).parts[1];
    assert_eq!(
        part.content_type.as_deref(),
        Some("image/png; charset=binary")
    );
    assert_eq!(
        part.file_name.as_deref(),
        Some("01234567-89ab-4def-8123-456789abcdef.png")
    );
}

#[tokio::test]
async fn a_content_type_without_a_slash_becomes_the_extension() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "plain", b"x"));
    fixture.script_upload(id_reply("file-5"));
    fixture.uploader.upload(IMAGE_URL).await;
    let uploads = fixture.uploads();
    let part = &first(&uploads).parts[1];
    assert_eq!(
        part.file_name.as_deref(),
        Some("01234567-89ab-4def-8123-456789abcdef.plain")
    );
}

#[tokio::test]
async fn an_empty_subtype_falls_back_to_jpg() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "image/", b"x"));
    fixture.script_upload(id_reply("file-6"));
    fixture.uploader.upload(IMAGE_URL).await;
    let uploads = fixture.uploads();
    let part = &first(&uploads).parts[1];
    assert_eq!(
        part.file_name.as_deref(),
        Some("01234567-89ab-4def-8123-456789abcdef.jpg")
    );
}

#[tokio::test]
async fn the_cached_id_is_returned_without_another_request() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file-1"));
    assert_eq!(
        fixture.uploader.upload(IMAGE_URL).await,
        Some(String::from("file-1"))
    );
    assert_eq!(
        fixture.uploader.upload(IMAGE_URL).await,
        Some(String::from("file-1"))
    );
    assert_eq!(fixture.downloads().len(), 1);
    assert_eq!(fixture.uploads().len(), 1);
}

#[tokio::test]
async fn a_different_url_is_uploaded_again() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_download(answer_with_type(200, "image/png", b"y"));
    fixture.script_upload(id_reply("file-1"));
    fixture.script_upload(id_reply("file-2"));
    assert_eq!(
        fixture.uploader.upload("https://img.test/a").await,
        Some(String::from("file-1"))
    );
    assert_eq!(
        fixture.uploader.upload("https://img.test/b").await,
        Some(String::from("file-2"))
    );
    assert_eq!(fixture.downloads().len(), 2);
    assert_eq!(fixture.uploads().len(), 2);
}

#[tokio::test]
async fn a_cached_id_does_not_ask_for_a_token_again() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file-1"));
    fixture.uploader.upload(IMAGE_URL).await;
    fixture.bearer.revoke();
    assert_eq!(
        fixture.uploader.upload(IMAGE_URL).await,
        Some(String::from("file-1"))
    );
    assert_eq!(fixture.bearer.seen().len(), 1);
}

#[tokio::test]
async fn the_cache_drops_the_oldest_entry_first() {
    let mut given = setup();
    given.capacity = 2;
    let fixture = fixture(given);
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file"));
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file"));
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file"));
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file"));
    fixture.uploader.upload("https://img.test/a").await;
    fixture.uploader.upload("https://img.test/b").await;
    fixture.uploader.upload("https://img.test/a").await;
    fixture.uploader.upload("https://img.test/c").await;
    fixture.uploader.upload("https://img.test/a").await;
    assert_eq!(fixture.downloads().len(), 4);
}

#[tokio::test]
async fn a_capacity_of_zero_keeps_nothing() {
    let mut given = setup();
    given.capacity = 0;
    let fixture = fixture(given);
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file"));
    fixture.script_upload(id_reply("file"));
    fixture.uploader.upload(IMAGE_URL).await;
    fixture.uploader.upload(IMAGE_URL).await;
    assert_eq!(fixture.downloads().len(), 2);
    assert_eq!(fixture.uploads().len(), 2);
}

#[tokio::test]
async fn an_upload_status_of_500_returns_none_and_is_not_cached() {
    let fixture = fixture(setup());
    fixture.script_upload(answer(500, r#"{"id":"file-500"}"#));
    assert_eq!(fixture.uploader.upload(DATA_URL).await, None);
    fixture.script_upload(id_reply("file-9"));
    assert_eq!(
        fixture.uploader.upload(DATA_URL).await,
        Some(String::from("file-9"))
    );
    assert_eq!(fixture.uploads().len(), 2);
}

#[tokio::test]
async fn a_download_status_of_404_returns_none() {
    let fixture = fixture(setup());
    fixture.script_download(answer(404, "missing"));
    assert_eq!(fixture.uploader.upload(IMAGE_URL).await, None);
    assert!(fixture.uploads().is_empty());
}

#[tokio::test]
async fn a_reply_without_an_id_returns_none_and_is_not_cached() {
    let fixture = fixture(setup());
    fixture.script_upload(answer(200, r#"{"object":"file"}"#));
    assert_eq!(fixture.uploader.upload(DATA_URL).await, None);
    fixture.script_upload(id_reply("file-3"));
    assert_eq!(
        fixture.uploader.upload(DATA_URL).await,
        Some(String::from("file-3"))
    );
    assert_eq!(fixture.uploads().len(), 2);
}

#[tokio::test]
async fn an_empty_id_returns_none_and_is_not_cached() {
    let fixture = fixture(setup());
    fixture.script_upload(answer(200, r#"{"id":""}"#));
    assert_eq!(fixture.uploader.upload(DATA_URL).await, None);
    fixture.script_upload(id_reply("file-4"));
    assert_eq!(
        fixture.uploader.upload(DATA_URL).await,
        Some(String::from("file-4"))
    );
    assert_eq!(fixture.uploads().len(), 2);
}

#[tokio::test]
async fn a_reply_that_is_not_json_returns_none() {
    let fixture = fixture(setup());
    fixture.script_upload(answer(200, "nope"));
    assert_eq!(fixture.uploader.upload(DATA_URL).await, None);
}

#[tokio::test]
async fn a_reply_with_a_non_string_id_returns_none() {
    let fixture = fixture(setup());
    fixture.script_upload(answer(200, r#"{"id":5}"#));
    assert_eq!(fixture.uploader.upload(DATA_URL).await, None);
}

#[tokio::test]
async fn a_failed_token_returns_none_without_uploading() {
    let mut given = setup();
    given.token = None;
    let fixture = fixture(given);
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    assert_eq!(fixture.uploader.upload(IMAGE_URL).await, None);
    assert_eq!(fixture.downloads().len(), 1);
    assert!(fixture.uploads().is_empty());
}

#[tokio::test]
async fn a_transport_download_failure_returns_none() {
    let fixture = fixture(setup());
    fixture.script_download(Scripted::Fail);
    assert_eq!(fixture.uploader.upload(IMAGE_URL).await, None);
    assert!(fixture.uploads().is_empty());
}

#[tokio::test]
async fn a_transport_upload_failure_returns_none_and_is_not_cached() {
    let fixture = fixture(setup());
    fixture.script_upload(Scripted::Fail);
    assert_eq!(fixture.uploader.upload(DATA_URL).await, None);
    fixture.script_upload(id_reply("file-5"));
    assert_eq!(
        fixture.uploader.upload(DATA_URL).await,
        Some(String::from("file-5"))
    );
}

#[tokio::test]
async fn the_upload_url_uses_the_environment_base() {
    let mut given = setup();
    given.api_base = None;
    given.env_base = Some("https://env.test");
    let fixture = fixture(given);
    fixture.script_upload(id_reply("file-6"));
    fixture.uploader.upload(DATA_URL).await;
    let uploads = fixture.uploads();
    assert_eq!(first(&uploads).url, "https://env.test/files");
}

#[tokio::test]
async fn a_base_that_is_not_a_url_returns_none() {
    let mut given = setup();
    given.api_base = Some("not a url");
    let fixture = fixture(given);
    assert_eq!(fixture.uploader.upload(DATA_URL).await, None);
    assert!(fixture.uploads().is_empty());
}

#[tokio::test]
async fn a_data_url_with_bad_padding_returns_none() {
    let fixture = fixture(setup());
    assert_eq!(
        fixture.uploader.upload("data:image/png;base64,QQ").await,
        None
    );
    assert!(fixture.downloads().is_empty());
    assert!(fixture.uploads().is_empty());
}

#[tokio::test]
async fn data_after_padding_returns_none() {
    let fixture = fixture(setup());
    assert_eq!(
        fixture.uploader.upload("data:image/png;base64,QU=D").await,
        None
    );
    assert!(fixture.uploads().is_empty());
}

#[tokio::test]
async fn a_padding_shorter_than_the_quad_needs_returns_none() {
    let fixture = fixture(setup());
    assert_eq!(
        fixture.uploader.upload("data:image/png;base64,QU=").await,
        None
    );
    assert!(fixture.uploads().is_empty());
}

#[tokio::test]
async fn padding_among_the_data_is_ignored() {
    let fixture = fixture(setup());
    fixture.script_upload(id_reply("file-16"));
    fixture.uploader.upload("data:image/png;base64,Q=UJD").await;
    let uploads = fixture.uploads();
    assert_eq!(first(&uploads).parts[1].data, b"ABC");
}

#[tokio::test]
async fn a_data_character_after_the_padding_is_refused() {
    let fixture = fixture(setup());
    assert_eq!(
        fixture.uploader.upload("data:image/png;base64,QU==x").await,
        None
    );
    assert!(fixture.uploads().is_empty());
}

#[tokio::test]
async fn a_three_character_payload_is_decoded() {
    let fixture = fixture(setup());
    fixture.script_upload(id_reply("file-15"));
    fixture.uploader.upload("data:image/png;base64,QUJ=").await;
    let uploads = fixture.uploads();
    assert_eq!(first(&uploads).parts[1].data, b"AB");
}

#[tokio::test]
async fn a_data_url_payload_stops_at_the_first_newline() {
    let fixture = fixture(setup());
    fixture.script_upload(id_reply("file-7"));
    fixture
        .uploader
        .upload("data:image/png;base64,QUJD\nREVG")
        .await;
    let uploads = fixture.uploads();
    assert_eq!(first(&uploads).parts[1].data, b"ABC");
}

#[tokio::test]
async fn whitespace_in_a_data_url_payload_is_ignored() {
    let fixture = fixture(setup());
    fixture.script_upload(id_reply("file-8"));
    fixture
        .uploader
        .upload("data:image/png;base64,QUJD REVG")
        .await;
    let uploads = fixture.uploads();
    assert_eq!(first(&uploads).parts[1].data, b"ABCDEF");
}

#[tokio::test]
async fn padding_beyond_the_canonical_form_is_accepted() {
    let fixture = fixture(setup());
    fixture.script_upload(id_reply("file-9"));
    fixture.uploader.upload("data:image/png;base64,QQ===").await;
    let uploads = fixture.uploads();
    assert_eq!(first(&uploads).parts[1].data, b"A");
}

#[tokio::test]
async fn a_data_url_without_the_base64_marker_is_downloaded() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file-10"));
    fixture.uploader.upload("data:image/png,xyz").await;
    let downloads = fixture.downloads();
    assert_eq!(first(&downloads).url, "data:image/png,xyz");
    assert_eq!(fixture.uploads().len(), 1);
}

#[tokio::test]
async fn a_data_url_parameter_before_the_marker_is_downloaded() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file-11"));
    fixture
        .uploader
        .upload("data:image/png;charset=x;base64,QUJD")
        .await;
    assert_eq!(fixture.downloads().len(), 1);
}

#[tokio::test]
async fn a_data_url_with_an_empty_mime_is_downloaded() {
    let fixture = fixture(setup());
    fixture.script_download(answer_with_type(200, "image/png", b"x"));
    fixture.script_upload(id_reply("file-12"));
    fixture.uploader.upload("data:;base64,QUJD").await;
    assert_eq!(fixture.downloads().len(), 1);
}

#[tokio::test]
async fn a_data_url_with_a_non_ascii_payload_returns_none() {
    let fixture = fixture(setup());
    assert_eq!(
        fixture
            .uploader
            .upload("data:image/png;base64,QUJD\u{e9}")
            .await,
        None
    );
    assert!(fixture.uploads().is_empty());
}

#[tokio::test]
async fn a_set_credential_is_passed_to_the_bearer() {
    let mut given = setup();
    given.credentials = Some("client:secret");
    let fixture = fixture(given);
    fixture.script_upload(id_reply("file-13"));
    fixture.uploader.upload(DATA_URL).await;
    assert_eq!(
        fixture.bearer.seen(),
        vec![Some(String::from("client:secret"))]
    );
}

#[tokio::test]
async fn an_empty_credential_is_not_passed_to_the_bearer() {
    let mut given = setup();
    given.credentials = Some("");
    let fixture = fixture(given);
    fixture.script_upload(id_reply("file-14"));
    fixture.uploader.upload(DATA_URL).await;
    assert_eq!(fixture.bearer.seen(), vec![None]);
}
