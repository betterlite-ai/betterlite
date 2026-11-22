#![cfg(test)]

use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::request::Parts;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use betterlite_mcp::byok_token_exchange::ports::{
    AuthCodeLookup, AuthCodeRecord, ByokCredentialSink, SinkError,
};
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{ByokOauthRoutes, RequestBaseUrl, routes};

pub(crate) const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
pub(crate) const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
pub(crate) const FORM_TYPE: &str = "application/x-www-form-urlencoded";

pub(crate) struct FixedBaseUrl(pub(crate) String);

impl RequestBaseUrl for FixedBaseUrl {
    fn base_url(&self, _parts: &Parts) -> String {
        self.0.clone()
    }
}

#[derive(Clone)]
pub(crate) struct FakeCodes {
    record: Option<AuthCodeRecord>,
    removed: Arc<AtomicBool>,
}

impl FakeCodes {
    pub(crate) fn new(record: Option<AuthCodeRecord>) -> Self {
        Self {
            record,
            removed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn removed(&self) -> bool {
        self.removed.load(Ordering::Acquire)
    }
}

impl AuthCodeLookup for FakeCodes {
    fn purge_expired(&self, _now: f64) {}

    fn get(&self, _code: &str) -> Option<AuthCodeRecord> {
        self.record.clone()
    }

    fn remove(&self, _code: &str) {
        self.removed.store(true, Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SinkCall {
    Store,
    Invalidate,
}

#[derive(Default)]
struct FakeSinkState {
    calls: Vec<SinkCall>,
    fail_store: bool,
    fail_invalidate: bool,
}

#[derive(Clone, Default)]
pub(crate) struct FakeSink {
    state: Arc<Mutex<FakeSinkState>>,
}

impl FakeSink {
    pub(crate) fn failing_store() -> Self {
        let sink = Self::default();
        // the state is shared with the clone inside the router, so the flag is set through the same Arc
        sink.lock().fail_store = true;
        sink
    }

    pub(crate) fn failing_invalidate() -> Self {
        let sink = Self::default();
        sink.lock().fail_invalidate = true;
        sink
    }

    pub(crate) fn calls(&self) -> Vec<SinkCall> {
        self.lock().calls.clone()
    }

    fn lock(&self) -> MutexGuard<'_, FakeSinkState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl FakeSink {
    fn finish(
        &self,
        call: SinkCall,
        failure: Option<SinkError>,
    ) -> impl Future<Output = Result<(), SinkError>> + Send {
        self.lock().calls.push(call);
        ready(failure.map_or(Ok(()), Err))
    }

    fn store_failure(&self) -> Option<SinkError> {
        self.lock().fail_store.then(|| SinkError::Store {
            message: "database unavailable".to_owned(),
        })
    }

    fn invalidate_failure(&self) -> Option<SinkError> {
        self.lock().fail_invalidate.then(|| SinkError::Invalidate {
            message: "peer publish failed".to_owned(),
        })
    }
}

impl ByokCredentialSink for FakeSink {
    fn store_credential(
        &self,
        _user_id: &str,
        _server_id: &str,
        _credential: &SecretString,
    ) -> impl Future<Output = Result<(), SinkError>> + Send {
        self.finish(SinkCall::Store, self.store_failure())
    }

    fn invalidate_cache(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> impl Future<Output = Result<(), SinkError>> + Send {
        self.finish(SinkCall::Invalidate, self.invalidate_failure())
    }
}

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(crate) fn master() -> SecretString {
    secret("sk-master")
}

pub(crate) fn record() -> AuthCodeRecord {
    AuthCodeRecord {
        api_key: secret("sk-user"),
        server_id: "srv-1".to_owned(),
        code_challenge: CHALLENGE.to_owned(),
        redirect_uri: String::new(),
        client_id: String::new(),
        user_id: "user-1".to_owned(),
        expires_at: 4_000_000_000.0,
    }
}

#[derive(Default)]
pub(crate) struct TestApp {
    pub(crate) record: Option<AuthCodeRecord>,
    pub(crate) credentials: Option<FakeSink>,
    pub(crate) master_key: Option<SecretString>,
    pub(crate) base_url: String,
    pub(crate) root_prefixes: Vec<String>,
    pub(crate) suffix: String,
}

impl TestApp {
    pub(crate) fn new(record: Option<AuthCodeRecord>) -> Self {
        let (base_url, root_prefixes, suffix) = Self::defaults();
        Self {
            record,
            credentials: Some(FakeSink::default()),
            master_key: Some(master()),
            base_url,
            root_prefixes,
            suffix,
        }
    }

    fn defaults() -> (String, Vec<String>, String) {
        ("https://proxy.test".to_owned(), Vec::new(), String::new())
    }

    pub(crate) fn with_record() -> Self {
        Self::new(Some(record()))
    }

    pub(crate) fn build(self) -> Built {
        let codes = FakeCodes::new(self.record);
        let router = routes(
            Arc::new(ByokOauthRoutes {
                codes: codes.clone(),
                credentials: self.credentials,
                master_key: self.master_key,
                base_url: FixedBaseUrl(self.base_url),
                root_prefixes: self.root_prefixes,
            }),
            &self.suffix,
        );
        Built { router, codes }
    }
}

pub(crate) struct Built {
    pub(crate) router: Router,
    pub(crate) codes: FakeCodes,
}

pub(crate) async fn send(router: Router, request: Request<Body>) -> Response {
    router.oneshot(request).await.unwrap()
}

pub(crate) fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

pub(crate) fn post_form(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", FORM_TYPE)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

pub(crate) async fn text(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), 0x0010_0000).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

pub(crate) fn header<'response>(
    response: &'response Response,
    name: &str,
) -> Option<&'response str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn a_prefix_equal_to_the_suffix_does_not_panic_and_the_suffix_wins() {
    let mut app = TestApp::with_record();
    app.base_url = "https://proxy.test/root".to_owned();
    app.root_prefixes = vec!["/prem".to_owned()];
    app.suffix = "/prem".to_owned();
    let response = send(
        app.build().router,
        get("/.well-known/oauth-authorization-server/prem/v1/mcp/oauth"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains(r#""issuer":"https://proxy.test/root/v1/mcp/oauth""#));
}

#[tokio::test]
async fn duplicate_root_prefixes_are_registered_once() {
    let mut app = TestApp::with_record();
    app.base_url = "https://proxy.test/root".to_owned();
    app.root_prefixes = vec!["/litellm".to_owned(), "/litellm".to_owned()];
    let response = send(
        app.build().router,
        get("/.well-known/oauth-authorization-server/litellm/v1/mcp/oauth"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains(r#""issuer":"https://proxy.test/litellm/v1/mcp/oauth""#));
}

#[tokio::test]
async fn an_unconfigured_well_known_prefix_is_not_found() {
    let mut app = TestApp::with_record();
    app.suffix = "/prem".to_owned();
    let response = send(
        app.build().router,
        get("/.well-known/oauth-authorization-server/other/v1/mcp/oauth"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_token_path_only_answers_post() {
    let response = send(
        TestApp::with_record().build().router,
        get("/v1/mcp/oauth/token"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
