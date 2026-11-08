#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::IntoFuture as _;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::document::{IdpDocument, decode_idp_document};
use super::{IdpClient, IdpError};
use crate::mcp_jwt_signer::jwks_document::get_jwks;
use crate::mcp_jwt_signer::signing_key::SigningKey;

type Server = JoinSet<Result<(), io::Error>>;

const LATER_BY_AN_HOUR: i64 = 1000 + 3599;
const HOUR_AND_ONE: i64 = 1000 + 3600;
const LATER_BY_A_DAY: i64 = 1000 + 86_399;
const DAY_AND_ONE: i64 = 1000 + 86_400;

#[derive(Clone)]
struct Answer {
    status: StatusCode,
    body: String,
}

#[derive(Default)]
struct Captured {
    paths: Vec<String>,
    methods: Vec<String>,
    accepts: Vec<String>,
    content_types: Vec<String>,
    bodies: Vec<Vec<u8>>,
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

async fn serve<F>(build_answers: F) -> (String, Arc<Mutex<Captured>>, Server)
where
    F: FnOnce(&str) -> BTreeMap<String, Answer>,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let answers = Arc::new(build_answers(&base));
    let captured = Arc::new(Mutex::new(Captured::default()));
    let handler_captured = Arc::clone(&captured);
    let handler = move |method: Method, headers: HeaderMap, uri: Uri, body: Bytes| {
        let request_log = Arc::clone(&handler_captured);
        let answers_for_request = Arc::clone(&answers);
        async move {
            let mut record = request_log.lock().expect("captured request");
            record.paths.push(uri.path().to_owned());
            record.methods.push(method.as_str().to_owned());
            record.accepts.push(header(&headers, "accept"));
            record.content_types.push(header(&headers, "content-type"));
            record.bodies.push(body.to_vec());
            drop(record);
            answers_for_request.get(uri.path()).map_or_else(
                || (StatusCode::NOT_FOUND, String::new()),
                |answer| (answer.status, answer.body.clone()),
            )
        }
    };
    let router = Router::new().fallback(handler);
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, captured, server)
}

fn ok(body: &str) -> Answer {
    Answer {
        status: StatusCode::OK,
        body: body.to_owned(),
    }
}

fn client() -> IdpClient {
    IdpClient {
        client: reqwest::Client::default(),
        timeout: None,
        jwks_cache: Mutex::new(BTreeMap::new()),
        discovery_cache: Mutex::new(BTreeMap::new()),
    }
}

fn idp_key() -> SigningKey {
    SigningKey::load(&BTreeMap::new()).expect("generated key")
}

fn jwks_body(key: &SigningKey) -> String {
    serde_json::to_string(&get_jwks(key)).expect("jwks json")
}

fn requests(captured: &Arc<Mutex<Captured>>) -> usize {
    captured.lock().expect("captured request").paths.len()
}

#[tokio::test]
async fn fetch_jwks_sends_the_json_accept_header_and_returns_the_keys() {
    let key = idp_key();
    let body = jwks_body(&key);
    let (base, captured, _server) =
        serve(|_base| BTreeMap::from([("/jwks".to_owned(), ok(&body))])).await;
    let keys = client()
        .fetch_jwks(&format!("{base}/jwks"), 1000)
        .await
        .expect("jwks");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].common.key_id.as_deref(), Some(key.kid.as_str()));
    let record = captured.lock().expect("captured request");
    assert_eq!(record.paths, ["/jwks"]);
    assert_eq!(record.accepts, ["application/json"]);
}

#[tokio::test]
async fn fetch_jwks_caches_each_uri_for_an_hour() {
    let key = idp_key();
    let body = jwks_body(&key);
    let (base, captured, _server) =
        serve(|_base| BTreeMap::from([("/jwks".to_owned(), ok(&body))])).await;
    let uri = format!("{base}/jwks");
    let client = client();
    client.fetch_jwks(&uri, 1000).await.expect("first");
    client
        .fetch_jwks(&uri, LATER_BY_AN_HOUR)
        .await
        .expect("cached");
    assert_eq!(requests(&captured), 1);
    client
        .fetch_jwks(&uri, HOUR_AND_ONE)
        .await
        .expect("expired");
    assert_eq!(requests(&captured), 2);
}

#[tokio::test]
async fn fetch_jwks_caches_each_uri_separately() {
    let key = idp_key();
    let body = jwks_body(&key);
    let (base, captured, _server) = serve(|_base| {
        BTreeMap::from([
            ("/jwks-a".to_owned(), ok(&body)),
            ("/jwks-b".to_owned(), ok(&body)),
        ])
    })
    .await;
    let client = client();
    client
        .fetch_jwks(&format!("{base}/jwks-a"), 1000)
        .await
        .expect("first uri");
    client
        .fetch_jwks(&format!("{base}/jwks-b"), 1000)
        .await
        .expect("second uri");
    assert_eq!(requests(&captured), 2);
}

#[tokio::test]
async fn fetch_jwks_fails_on_a_non_success_status() {
    let (base, _captured, _server) = serve(|_base| {
        BTreeMap::from([(
            "/jwks".to_owned(),
            Answer {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: "{}".to_owned(),
            },
        )])
    })
    .await;
    let error = client()
        .fetch_jwks(&format!("{base}/jwks"), 1000)
        .await
        .expect_err("status error");
    assert!(matches!(error, IdpError::Status { .. }));
    assert!(error.to_string().contains("500"));
}

#[tokio::test]
async fn fetch_jwks_returns_no_keys_when_the_body_has_none() {
    let (base, _captured, _server) =
        serve(|_base| BTreeMap::from([("/jwks".to_owned(), ok("{}"))])).await;
    let keys = client()
        .fetch_jwks(&format!("{base}/jwks"), 1000)
        .await
        .expect("jwks");
    assert!(keys.is_empty());
}

#[tokio::test]
async fn fetch_oidc_discovery_is_never_cached() {
    let (base, captured, _server) = serve(|_base| {
        BTreeMap::from([(
            "/discovery".to_owned(),
            ok(r#"{"jwks_uri":"https://idp/jwks"}"#),
        )])
    })
    .await;
    let client = client();
    let uri = format!("{base}/discovery");
    client.fetch_oidc_discovery(&uri).await.expect("first");
    client.fetch_oidc_discovery(&uri).await.expect("second");
    assert_eq!(requests(&captured), 2);
    let record = captured.lock().expect("captured request");
    assert_eq!(record.accepts, ["application/json", "application/json"]);
}

#[tokio::test]
async fn get_oidc_discovery_caches_a_document_with_a_jwks_uri_for_a_day() {
    let (base, captured, _server) = serve(|base| {
        BTreeMap::from([(
            "/discovery".to_owned(),
            ok(&format!(r#"{{"jwks_uri":"{base}/jwks"}}"#)),
        )])
    })
    .await;
    let client = client();
    let uri = format!("{base}/discovery");
    let first = client
        .get_oidc_discovery(Some(&uri), 1000)
        .await
        .expect("first");
    assert_eq!(
        first.jwks_uri.as_deref(),
        Some(format!("{base}/jwks").as_str())
    );
    client
        .get_oidc_discovery(Some(&uri), LATER_BY_A_DAY)
        .await
        .expect("cached");
    assert_eq!(requests(&captured), 1);
    client
        .get_oidc_discovery(Some(&uri), DAY_AND_ONE)
        .await
        .expect("expired");
    assert_eq!(requests(&captured), 2);
}

#[tokio::test]
async fn get_oidc_discovery_does_not_cache_a_document_without_a_jwks_uri() {
    let (base, captured, _server) =
        serve(|_base| BTreeMap::from([("/discovery".to_owned(), ok("{}"))])).await;
    let client = client();
    let uri = format!("{base}/discovery");
    let document = client
        .get_oidc_discovery(Some(&uri), 1000)
        .await
        .expect("first");
    assert_eq!(document.jwks_uri, None);
    client
        .get_oidc_discovery(Some(&uri), 1000)
        .await
        .expect("second");
    assert_eq!(requests(&captured), 2);
}

#[tokio::test]
async fn get_oidc_discovery_without_a_uri_is_an_empty_document() {
    let client = client();
    let document = client.get_oidc_discovery(None, 1000).await.expect("empty");
    assert_eq!(document.jwks_uri, None);
}

#[tokio::test]
async fn introspect_posts_the_token_form_field_and_returns_the_claims() {
    let body = r#"{"active":true,"sub":"alice","scope":"mcp:tools/list"}"#;
    let (base, captured, _server) =
        serve(|_base| BTreeMap::from([("/introspect".to_owned(), ok(body))])).await;
    let claims = client()
        .introspect(
            Some(&format!("{base}/introspect")),
            &SecretString::from("opaque-token"),
        )
        .await
        .expect("introspection");
    assert_eq!(
        claims.get("sub").and_then(|value| value.as_str()),
        Some("alice")
    );
    let record = captured.lock().expect("captured request");
    assert_eq!(record.methods, ["POST"]);
    assert_eq!(record.accepts, ["application/json"]);
    assert!(record.content_types[0].starts_with("application/x-www-form-urlencoded"));
    assert_eq!(record.bodies[0], b"token=opaque-token");
}

#[tokio::test]
async fn introspect_rejects_an_inactive_token() {
    let (base, _captured, _server) =
        serve(|_base| BTreeMap::from([("/introspect".to_owned(), ok(r#"{"active":false}"#))]))
            .await;
    let error = client()
        .introspect(
            Some(&format!("{base}/introspect")),
            &SecretString::from("opaque-token"),
        )
        .await
        .expect_err("inactive");
    assert!(matches!(error, IdpError::InactiveToken));
    assert_eq!(
        error.to_string(),
        "incoming token is inactive (introspection returned active=false)"
    );
}

#[tokio::test]
async fn introspect_rejects_a_response_without_an_active_field() {
    let (base, _captured, _server) =
        serve(|_base| BTreeMap::from([("/introspect".to_owned(), ok(r#"{"sub":"alice"}"#))])).await;
    let error = client()
        .introspect(
            Some(&format!("{base}/introspect")),
            &SecretString::from("opaque-token"),
        )
        .await
        .expect_err("inactive");
    assert!(matches!(error, IdpError::InactiveToken));
}

#[tokio::test]
async fn introspect_without_an_endpoint_is_an_error() {
    let error = client()
        .introspect(None, &SecretString::from("opaque-token"))
        .await
        .expect_err("no endpoint");
    assert_eq!(
        error.to_string(),
        "token_introspection_endpoint is required for opaque token verification but is not configured"
    );
}

#[tokio::test]
async fn introspect_fails_on_a_non_success_status() {
    let (base, _captured, _server) = serve(|_base| {
        BTreeMap::from([(
            "/introspect".to_owned(),
            Answer {
                status: StatusCode::UNAUTHORIZED,
                body: "{}".to_owned(),
            },
        )])
    })
    .await;
    let error = client()
        .introspect(
            Some(&format!("{base}/introspect")),
            &SecretString::from("opaque-token"),
        )
        .await
        .expect_err("status error");
    assert!(matches!(error, IdpError::Status { .. }));
}

#[test]
fn decode_idp_document_reads_each_shape() {
    let discovery: IdpDocument =
        decode_idp_document(br#"{"jwks_uri":"https://idp/jwks"}"#).expect("discovery");
    assert!(matches!(
        &discovery,
        IdpDocument::Discovery(document) if document.jwks_uri.as_deref() == Some("https://idp/jwks")
    ));
    let jwks =
        decode_idp_document(br#"{"keys":[{"kty":"RSA","n":"AQAB","e":"AQAB"}]}"#).expect("jwks");
    assert!(matches!(
        &jwks,
        IdpDocument::Jwks(document) if document.keys.as_ref().is_some_and(|keys| keys.len() == 1)
    ));
    let introspection = decode_idp_document(br#"{"active":true,"sub":"alice"}"#).expect("intro");
    assert!(matches!(
        &introspection,
        IdpDocument::Introspection(response) if response.active == Some(true)
    ));
}

#[test]
fn decode_idp_document_rejects_bytes_that_are_not_a_json_object() {
    drop(decode_idp_document(b"[]").expect_err("an array is not a document"));
    drop(decode_idp_document(b"not json").expect_err("not json"));
}

#[test]
fn replays_the_mcp_jwt_idp_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/mcp_jwt_idp_document");
    let paths: Vec<_> = fs::read_dir(corpus).expect("corpus").collect();
    assert!(!paths.is_empty(), "the corpus has no seed files");
    let documents: Vec<_> = paths
        .into_iter()
        .map(|entry| {
            let bytes = fs::read(entry.expect("entry").path()).expect("seed file");
            decode_idp_document(&bytes)
        })
        .collect();
    assert!(documents.iter().any(Result::is_ok));
}

#[test]
fn the_caches_use_litellms_ttls() {
    assert_eq!(super::JWKS_CACHE_SECONDS, 3600);
    assert_eq!(super::DISCOVERY_CACHE_SECONDS, 86_400);
}
