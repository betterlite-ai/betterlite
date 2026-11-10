#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse, base_builder};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, Method};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::error::CacheLookupError;
use super::lookup::{MAX_PAGINATION_PAGES, check_cache};
use super::records::{CACHE_PAGE_LIMIT, CachedContent, decode_cached_page};
use super::target::{CacheProvider, CacheTargetInput, cache_target};
use super::transport::{CacheTransport, cache_headers, cache_request};
use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::target::{AuthHeader, EndpointTarget};

const DEADLINE: Duration = Duration::from_secs(5);

const VERTEX_TARGET_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/cachedContents";

const PROXY_TARGET_URL: &str =
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/cachedContents";

#[derive(Clone)]
enum Reply {
    Status { status: u16, body: Vec<u8> },
    Failure,
}

#[derive(Clone)]
struct Recorded {
    method: Method,
    url: String,
    headers: HeaderMap,
}

struct Pages {
    replies: Mutex<VecDeque<Reply>>,
    fallback: Option<Reply>,
    requests: Mutex<Vec<Recorded>>,
}

impl Pages {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            fallback: None,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn repeating(reply: Reply) -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            fallback: Some(reply),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    fn urls(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|record| record.url)
            .collect()
    }
}

impl CacheTransport for Pages {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let url = request.url.to_string();
        let headers = request.headers;
        self.requests.lock().unwrap().push(Recorded {
            method: request.method,
            url,
            headers,
        });
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .or_else(|| self.fallback.clone())
            .unwrap();
        ready(match reply {
            Reply::Status { status, body } => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Reply::Failure => Err(TransportError::Closed),
        })
    }
}

fn page(body: &str) -> Reply {
    Reply::Status {
        status: 200,
        body: body.as_bytes().to_vec(),
    }
}

fn status(status: u16, body: &str) -> Reply {
    Reply::Status {
        status,
        body: body.as_bytes().to_vec(),
    }
}

fn list_page(display_name: &str, next: Option<&str>) -> Reply {
    let token = next.map_or_else(String::new, |token| {
        format!(r#","nextPageToken":"{token}""#)
    });
    page(&format!(
        r#"{{"cachedContents":[{{"displayName":"{display_name}","name":"name-{display_name}"}}]{token}}}"#
    ))
}

fn vertex_input<'text>(
    api_base: Option<&'text str>,
    token: &'text SecretString,
) -> CacheTargetInput<'text> {
    CacheTargetInput {
        provider: CacheProvider::Vertex,
        api_base,
        project: Some("proj"),
        location: Some("us-central1"),
        auth_token: Some(token),
        gemini_api_key: None,
        model: None,
    }
}

fn native_target() -> EndpointTarget {
    EndpointTarget {
        url: VERTEX_TARGET_URL.to_owned(),
        auth: Some(AuthHeader::Token(SecretString::from("token-abc"))),
    }
}

fn proxy_target() -> EndpointTarget {
    EndpointTarget {
        url: format!("{PROXY_TARGET_URL}?x=1"),
        auth: None,
    }
}

struct Redirecting {
    base: String,
    paths: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<()>,
}

async fn redirecting_server() -> Redirecting {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let paths: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_redirects(listener, Arc::clone(&paths)));
    Redirecting {
        base,
        paths,
        _tasks: tasks,
    }
}

async fn serve_redirects(listener: TcpListener, paths: Arc<Mutex<Vec<String>>>) {
    loop {
        let Ok((mut stream, _peer)) = listener.accept().await else {
            return;
        };
        let Some(path) = request_path(&mut stream).await else {
            continue;
        };
        paths.lock().unwrap().push(path.clone());
        let response = if path == "/redirected" {
            page_response()
        } else {
            redirect_response()
        };
        let _written = stream.write_all(response.as_bytes()).await;
    }
}

async fn request_path(stream: &mut TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.ok()?;
    line.split_whitespace().nth(1).map(str::to_owned)
}

fn page_response() -> String {
    let body = r#"{"cachedContents":[{"displayName":"key-1","name":"name-1"}]}"#;
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn redirect_response() -> String {
    "HTTP/1.1 307 Temporary Redirect\r\nlocation: /redirected\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        .to_owned()
}

#[rstest]
#[case::regional("us-central1", "https://us-central1-aiplatform.googleapis.com")]
#[case::global("global", "https://aiplatform.googleapis.com")]
#[case::multi_region("us", "https://aiplatform.us.rep.googleapis.com")]
fn vertex_target_uses_the_location_host(#[case] location: &str, #[case] host: &str) {
    let token = SecretString::from("token-abc");
    let mut input = vertex_input(None, &token);
    input.location = Some(location);
    let target = cache_target(&input).unwrap();
    assert_eq!(
        target.url,
        format!("{host}/v1/projects/proj/locations/{location}/cachedContents")
    );
    assert!(matches!(target.auth, Some(AuthHeader::Token(_))));
}

#[test]
fn beta_target_uses_v1beta1() {
    let token = SecretString::from("token-abc");
    let input = CacheTargetInput {
        provider: CacheProvider::VertexBeta,
        api_base: None,
        project: Some("proj"),
        location: Some("us-central1"),
        auth_token: Some(&token),
        gemini_api_key: None,
        model: None,
    };
    let target = cache_target(&input).unwrap();
    assert_eq!(target.url, VERTEX_TARGET_URL.replace("/v1/", "/v1beta1/"));
}

#[test]
fn gemini_target_uses_v1beta_and_the_api_key_header() {
    let key = SecretString::from("studio-key");
    let input = CacheTargetInput {
        provider: CacheProvider::Gemini,
        api_base: None,
        project: None,
        location: None,
        auth_token: None,
        gemini_api_key: Some(&key),
        model: None,
    };
    let target = cache_target(&input).unwrap();
    assert_eq!(
        target.url,
        "https://generativelanguage.googleapis.com/v1beta/cachedContents"
    );
    let Some(AuthHeader::Headers(headers)) = target.auth else {
        panic!("the Gemini target carries a header map");
    };
    assert_eq!(headers["x-goog-api-key"].expose_secret(), "studio-key");
}

#[test]
fn gemini_target_requires_the_api_key() {
    let input = CacheTargetInput {
        provider: CacheProvider::Gemini,
        api_base: None,
        project: None,
        location: None,
        auth_token: None,
        gemini_api_key: None,
        model: None,
    };
    assert!(matches!(
        cache_target(&input),
        Err(CacheLookupError::Target(EndpointTargetError::GeminiApiKey))
    ));
}

#[rstest]
#[case::v1(
    "https://proxy.example.com/v1",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/cachedContents"
)]
#[case::v1beta1(
    "https://proxy.example.com/v1beta1",
    "https://proxy.example.com/v1beta1/projects/proj/locations/us-central1/cachedContents"
)]
fn custom_version_root_grafts_the_default_path(#[case] api_base: &str, #[case] expected: &str) {
    let token = SecretString::from("token-abc");
    let input = vertex_input(Some(api_base), &token);
    let target = cache_target(&input).unwrap();
    assert_eq!(target.url, expected);
}

#[test]
fn existing_query_survives_grafting() {
    let token = SecretString::from("token-abc");
    let input = vertex_input(Some("https://proxy.example.com/v1?x=1"), &token);
    let target = cache_target(&input).unwrap();
    assert_eq!(target.url, format!("{PROXY_TARGET_URL}?x=1"));
}

#[test]
fn host_only_custom_base_appends_the_default_path() {
    let token = SecretString::from("token-abc");
    let input = vertex_input(Some("https://proxy.example.com"), &token);
    let target = cache_target(&input).unwrap();
    assert_eq!(target.url, PROXY_TARGET_URL);
}

#[test]
fn proxy_path_appends_the_cache_endpoint() {
    let token = SecretString::from("token-abc");
    let input = vertex_input(Some("https://proxy.example.com/foo"), &token);
    let target = cache_target(&input).unwrap();
    assert_eq!(target.url, "https://proxy.example.com/foo:cachedContents");
}

#[test]
fn empty_api_base_is_treated_as_absent() {
    let token = SecretString::from("token-abc");
    let input = vertex_input(Some(""), &token);
    let target = cache_target(&input).unwrap();
    assert_eq!(target.url, VERTEX_TARGET_URL);
}

#[test]
fn gemini_custom_base_requires_a_model() {
    let key = SecretString::from("studio-key");
    let input = CacheTargetInput {
        provider: CacheProvider::Gemini,
        api_base: Some("https://proxy.example.com"),
        project: None,
        location: None,
        auth_token: None,
        gemini_api_key: Some(&key),
        model: None,
    };
    assert!(matches!(
        cache_target(&input),
        Err(CacheLookupError::Target(EndpointTargetError::GeminiModel))
    ));
}

#[test]
fn gemini_custom_base_builds_the_model_path() {
    let key = SecretString::from("studio-key");
    let input = CacheTargetInput {
        provider: CacheProvider::Gemini,
        api_base: Some("https://proxy.example.com"),
        project: None,
        location: None,
        auth_token: None,
        gemini_api_key: Some(&key),
        model: Some("gemini-2.5-flash"),
    };
    let target = cache_target(&input).unwrap();
    assert_eq!(
        target.url,
        "https://proxy.example.com/models/gemini-2.5-flash:cachedContents"
    );
}

#[test]
fn missing_location_is_an_error() {
    let token = SecretString::from("token-abc");
    let mut input = vertex_input(None, &token);
    input.location = None;
    let error = cache_target(&input).unwrap_err();
    assert!(matches!(
        error,
        CacheLookupError::Target(EndpointTargetError::Location)
    ));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn invalid_location_is_an_error() {
    let token = SecretString::from("token-abc");
    let mut input = vertex_input(None, &token);
    input.location = Some("attacker.example/");
    assert!(matches!(
        cache_target(&input),
        Err(CacheLookupError::Target(EndpointTargetError::VertexUrl(_)))
    ));
}

#[test]
fn missing_project_is_an_error() {
    let token = SecretString::from("token-abc");
    let mut input = vertex_input(None, &token);
    input.project = None;
    assert!(matches!(
        cache_target(&input),
        Err(CacheLookupError::Target(EndpointTargetError::Project))
    ));
}

#[test]
fn empty_project_is_used_as_given() {
    let token = SecretString::from("token-abc");
    let mut input = vertex_input(None, &token);
    input.project = Some("");
    let target = cache_target(&input).unwrap();
    assert_eq!(
        target.url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects//locations/us-central1/cachedContents"
    );
}

#[test]
fn absent_auth_stays_absent() {
    let token = SecretString::from("token-abc");
    let mut input = vertex_input(None, &token);
    input.auth_token = None;
    let target = cache_target(&input).unwrap();
    assert!(target.auth.is_none());
}

#[test]
fn cache_headers_carry_content_type_and_a_bearer_token() {
    let target = native_target();
    let headers = cache_headers(&target, &BTreeMap::new()).unwrap();
    assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
    let authorization = headers.get(AUTHORIZATION).unwrap();
    assert_eq!(authorization.to_str().unwrap(), "Bearer token-abc");
    assert!(authorization.is_sensitive());
}

#[test]
fn cache_headers_carry_the_gemini_key() {
    let key = SecretString::from("studio-key");
    let target = EndpointTarget {
        url: "https://generativelanguage.googleapis.com/v1beta/cachedContents".to_owned(),
        auth: Some(AuthHeader::Headers(BTreeMap::from([(
            "x-goog-api-key".to_owned(),
            key,
        )]))),
    };
    let headers = cache_headers(&target, &BTreeMap::new()).unwrap();
    let value = headers.get("x-goog-api-key").unwrap();
    assert_eq!(value.to_str().unwrap(), "studio-key");
    assert!(value.is_sensitive());
}

#[test]
fn cache_headers_apply_extras_last() {
    let target = native_target();
    let extras = BTreeMap::from([(
        AUTHORIZATION.as_str().to_owned(),
        "Bearer override".to_owned(),
    )]);
    let headers = cache_headers(&target, &extras).unwrap();
    assert_eq!(
        headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer override"
    );
}

#[test]
fn cache_headers_skip_absent_auth() {
    let target = proxy_target();
    let headers = cache_headers(&target, &BTreeMap::new()).unwrap();
    assert!(headers.get(AUTHORIZATION).is_none());
}

#[test]
fn cache_headers_reject_an_invalid_name() {
    let target = native_target();
    let extras = BTreeMap::from([("bad header".to_owned(), "value".to_owned())]);
    assert!(matches!(
        cache_headers(&target, &extras),
        Err(CacheLookupError::Header { .. })
    ));
}

#[test]
fn page_token_is_appended_after_an_existing_query() {
    let target = proxy_target();
    let request = cache_request(&target, Some("next-1"), &BTreeMap::new(), DEADLINE).unwrap();
    assert_eq!(
        request.url.as_str(),
        format!("{PROXY_TARGET_URL}?x=1&pageToken=next-1")
    );
}

#[test]
fn page_token_uses_a_question_mark_without_a_query() {
    let target = native_target();
    let request = cache_request(&target, Some("next"), &BTreeMap::new(), DEADLINE).unwrap();
    assert_eq!(
        request.url.as_str(),
        format!("{VERTEX_TARGET_URL}?pageToken=next")
    );
}

#[test]
fn absent_or_empty_page_token_keeps_the_base_url() {
    let target = native_target();
    let absent = cache_request(&target, None, &BTreeMap::new(), DEADLINE).unwrap();
    let empty = cache_request(&target, Some(""), &BTreeMap::new(), DEADLINE).unwrap();
    assert_eq!(absent.url.as_str(), VERTEX_TARGET_URL);
    assert_eq!(empty.url.as_str(), VERTEX_TARGET_URL);
}

#[test]
fn invalid_target_url_is_an_error() {
    let target = EndpointTarget {
        url: "not a url".to_owned(),
        auth: None,
    };
    let error = cache_request(&target, None, &BTreeMap::new(), DEADLINE).unwrap_err();
    assert!(matches!(error, CacheLookupError::Url { .. }));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn first_page_hit() {
    let pages = Pages::new(vec![page(
        r#"{"cachedContents": [{"displayName": "key-1", "name": "name-1"}]}"#,
    )]);
    let found = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap();
    assert_eq!(found.as_deref(), Some("name-1"));
    assert_eq!(pages.requests().len(), 1);
}

#[tokio::test]
async fn second_page_hit() {
    let pages = Pages::new(vec![
        list_page("key-1", Some("next-1")),
        list_page("key-2", None),
    ]);
    let found = check_cache(&pages, &proxy_target(), "key-2", &BTreeMap::new(), DEADLINE)
        .await
        .unwrap();
    assert_eq!(found.as_deref(), Some("name-key-2"));
    let requests = pages.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url, format!("{PROXY_TARGET_URL}?x=1"));
    assert_eq!(
        requests[1].url,
        format!("{PROXY_TARGET_URL}?x=1&pageToken=next-1")
    );
}

#[tokio::test]
async fn list_request_carries_auth_and_extras() {
    let pages = Pages::new(vec![page(r#"{"cachedContents": []}"#)]);
    let extras = BTreeMap::from([("x-extra".to_owned(), "extra-value".to_owned())]);
    let found = check_cache(&pages, &native_target(), "key-1", &extras, DEADLINE)
        .await
        .unwrap();
    assert!(found.is_none());
    let request = &pages.requests()[0];
    assert_eq!(request.method, Method::GET);
    let headers = &request.headers;
    assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
    assert_eq!(
        headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer token-abc"
    );
    assert_eq!(headers.get("x-extra").unwrap(), "extra-value");
}

#[tokio::test]
async fn redirecting_list_page_is_followed() {
    let server = redirecting_server().await;
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap();
    let target = EndpointTarget {
        url: format!(
            "{}/v1/projects/proj/locations/us-central1/cachedContents",
            server.base
        ),
        auth: Some(AuthHeader::Token(SecretString::from("token-abc"))),
    };
    let found = check_cache(&client, &target, "key-1", &BTreeMap::new(), DEADLINE)
        .await
        .unwrap();
    assert_eq!(found.as_deref(), Some("name-1"));
    let paths = server.paths.lock().unwrap().clone();
    assert_eq!(
        paths,
        vec![
            "/v1/projects/proj/locations/us-central1/cachedContents".to_owned(),
            "/redirected".to_owned(),
        ]
    );
}

#[tokio::test]
async fn forbidden_is_miss() {
    let pages = Pages::new(vec![status(403, "no permission")]);
    let found = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap();
    assert!(found.is_none());
    assert_eq!(pages.requests().len(), 1);
}

#[tokio::test]
async fn missing_items_stops() {
    let pages = Pages::new(vec![page(r#"{"nextPageToken":"next-1"}"#)]);
    let found = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap();
    assert!(found.is_none());
    assert_eq!(pages.requests().len(), 1);
}

#[tokio::test]
async fn null_items_stops() {
    let pages = Pages::new(vec![page(
        r#"{"cachedContents":null,"nextPageToken":"next-1"}"#,
    )]);
    let found = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap();
    assert!(found.is_none());
    assert_eq!(pages.requests().len(), 1);
}

#[tokio::test]
async fn empty_page_token_stops() {
    let pages = Pages::new(vec![page(r#"{"cachedContents":[],"nextPageToken":""}"#)]);
    let found = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap();
    assert!(found.is_none());
    assert_eq!(pages.requests().len(), 1);
}

#[tokio::test]
async fn page_limit() {
    let pages = Pages::repeating(page(r#"{"cachedContents":[],"nextPageToken":"loop"}"#));
    let found = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap();
    assert!(found.is_none());
    assert_eq!(MAX_PAGINATION_PAGES, 100);
    let urls = pages.urls();
    assert_eq!(urls.len(), MAX_PAGINATION_PAGES);
    assert_eq!(urls[0], VERTEX_TARGET_URL);
    assert_eq!(urls[1], format!("{VERTEX_TARGET_URL}?pageToken=loop"));
    assert_eq!(
        urls[MAX_PAGINATION_PAGES - 1],
        format!("{VERTEX_TARGET_URL}?pageToken=loop")
    );
}

#[tokio::test]
async fn non403_error() {
    let pages = Pages::new(vec![status(429, "slow down")]);
    let error = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, CacheLookupError::Http { status: 429, .. }));
    assert_eq!(error.status_code(), 429);
    assert!(error.to_string().contains("slow down"));
    assert_eq!(pages.requests().len(), 1);
}

#[tokio::test]
async fn non_utf8_error_body_is_described() {
    let pages = Pages::new(vec![Reply::Status {
        status: 500,
        body: vec![0xff, 0xfe],
    }]);
    let error = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("2 bytes that are not UTF-8"));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn transport_failure_is_500() {
    let pages = Pages::new(vec![Reply::Failure]);
    let error = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, CacheLookupError::Transport { .. }));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn bad_page_json_is_a_decode_error() {
    let pages = Pages::new(vec![page("not json")]);
    let error = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, CacheLookupError::Decode { .. }));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn list_page_at_the_limit_is_decoded() {
    let pages = Pages::new(vec![Reply::Status {
        status: 200,
        body: vec![b' '; CACHE_PAGE_LIMIT],
    }]);
    let error = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, CacheLookupError::Decode { .. }));
}

#[tokio::test]
async fn oversized_list_page_is_refused() {
    let pages = Pages::new(vec![Reply::Status {
        status: 200,
        body: vec![b' '; CACHE_PAGE_LIMIT + 1],
    }]);
    let error = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, CacheLookupError::PageTooLarge { .. }));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn oversized_error_body_is_refused() {
    let pages = Pages::new(vec![Reply::Status {
        status: 500,
        body: vec![b' '; CACHE_PAGE_LIMIT + 1],
    }]);
    let error = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, CacheLookupError::PageTooLarge { .. }));
}

#[rstest]
#[case::exact("key-1", Some("name-key-1"))]
#[case::longer("key-10", None)]
#[case::shorter("key-", None)]
#[case::case_differs("Key-1", None)]
#[tokio::test]
async fn display_name_cases(#[case] key: &str, #[case] expected: Option<&str>) {
    let pages = Pages::new(vec![list_page("key-1", None)]);
    let found = check_cache(&pages, &native_target(), key, &BTreeMap::new(), DEADLINE)
        .await
        .unwrap();
    assert_eq!(found.as_deref(), expected);
}

#[tokio::test]
async fn matched_name_absent_is_a_miss() {
    let pages = Pages::new(vec![page(
        r#"{"cachedContents":[{"displayName":"key-1"}],"nextPageToken":"next-1"}"#,
    )]);
    let found = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap();
    assert!(found.is_none());
    assert_eq!(pages.requests().len(), 1);
}

#[tokio::test]
async fn matched_empty_name_is_returned() {
    let pages = Pages::new(vec![page(
        r#"{"cachedContents":[{"displayName":"key-1","name":""}]}"#,
    )]);
    let found = check_cache(
        &pages,
        &native_target(),
        "key-1",
        &BTreeMap::new(),
        DEADLINE,
    )
    .await
    .unwrap();
    assert_eq!(found.as_deref(), Some(""));
}

#[test]
fn decode_cached_page_reads_names_and_tokens() {
    let decoded = decode_cached_page(
        br#"{"cachedContents":[{"name":"n1","displayName":"d1","model":"m"}],"nextPageToken":"t"}"#,
    )
    .unwrap();
    assert_eq!(
        decoded.cached_contents,
        Some(vec![CachedContent {
            name: Some("n1".to_owned()),
            display_name: Some("d1".to_owned()),
            model: Some("m".to_owned()),
        }])
    );
    assert_eq!(decoded.next_page_token.as_deref(), Some("t"));
}

#[test]
fn absent_names_and_tokens_stay_absent() {
    let decoded = decode_cached_page(br#"{"cachedContents":[{"model":"m"}]}"#).unwrap();
    assert_eq!(
        decoded.cached_contents,
        Some(vec![CachedContent {
            name: None,
            display_name: None,
            model: Some("m".to_owned()),
        }])
    );
    assert!(decoded.next_page_token.is_none());
}

#[test]
fn nulls_take_the_absent_default() {
    let decoded = decode_cached_page(br#"{"cachedContents":null,"nextPageToken":null}"#).unwrap();
    assert!(decoded.cached_contents.is_none());
    assert!(decoded.next_page_token.is_none());
}

#[rstest]
#[case::items_object(r#"{"cachedContents":{}}"#)]
#[case::items_text(r#"{"cachedContents":"x"}"#)]
#[case::name_number(r#"{"cachedContents":[{"name":5}]}"#)]
#[case::token_number(r#"{"nextPageToken":5}"#)]
#[case::not_json("not json")]
fn wrong_page_types_are_rejected(#[case] body: &str) {
    assert!(matches!(
        decode_cached_page(body.as_bytes()),
        Err(CacheLookupError::Decode { .. })
    ));
}

#[test]
fn page_size_limit_boundary() {
    assert_eq!(CACHE_PAGE_LIMIT, 4 * 1024 * 1024);
    let at_limit = vec![b' '; CACHE_PAGE_LIMIT];
    assert!(matches!(
        decode_cached_page(&at_limit),
        Err(CacheLookupError::Decode { .. })
    ));
    let over_limit = vec![b' '; CACHE_PAGE_LIMIT + 1];
    assert!(matches!(
        decode_cached_page(&over_limit),
        Err(CacheLookupError::PageTooLarge {
            limit: CACHE_PAGE_LIMIT
        })
    ));
}

#[test]
fn replays_the_cache_page_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cache_page");
    fs::read_dir(corpus)
        .expect("corpus folder")
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .expect("utf-8 file name");
            let decoded = decode_cached_page(&bytes);
            if name.starts_with("valid") {
                decoded.expect("this seed is a cache page");
            } else {
                assert!(decoded.is_err(), "{name}");
            }
            Ok::<(), io::Error>(())
        })
        .expect("readable corpus");
}
