#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::{self, ErrorKind, Read as _, Write as _};
use std::net::{TcpListener as StdTcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::tls::Verification;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::Acceptor;
use rustls::{ServerConfig, StreamOwned};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::IbmDetectorClient;
use super::error::IbmClientError;
use crate::ibm_detector_client::config::IbmDetectorConfig;

const SERVER_CERT: &[u8] =
    include_bytes!("../../../../../tools/postgres-fixture/fixtures/tls/server.pem");
const SERVER_KEY: &[u8] =
    include_bytes!("../../../../../tools/postgres-fixture/fixtures/tls/server.key");
const TLS_TIMEOUT: Duration = Duration::from_secs(5);

struct CapturedRequest {
    headers: HeaderMap,
    body: String,
}

#[derive(serde::Deserialize)]
struct DetectorServerBody {
    contents: Vec<String>,
    detector_params: BTreeMap<String, GuardrailValue>,
}

#[derive(serde::Deserialize)]
struct OrchestratorBody {
    content: String,
    detectors: BTreeMap<String, BTreeMap<String, GuardrailValue>>,
}

async fn capturing_server(
    path: &'static str,
    status: StatusCode,
    body: String,
    capacity: usize,
) -> (
    String,
    mpsc::Receiver<CapturedRequest>,
    JoinSet<io::Result<()>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = mpsc::channel(capacity);
    let router = Router::new().route(
        path,
        post(move |headers: HeaderMap, request_body: String| {
            let request_tx = tx.clone();
            let reply = body.clone();
            async move {
                drop(
                    request_tx
                        .send(CapturedRequest {
                            headers,
                            body: request_body,
                        })
                        .await,
                );
                (status, reply)
            }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, rx, server)
}

fn config(is_detector_server: bool, api_url: &str) -> IbmDetectorConfig {
    IbmDetectorConfig {
        guardrail_name: "ibm".to_owned(),
        auth_token: SecretString::from("token"),
        base_url: "https://detector.test".to_owned(),
        api_url: api_url.to_owned(),
        detector_id: "jailbreak".to_owned(),
        is_detector_server,
        detector_params: BTreeMap::from([(
            "threshold".to_owned(),
            GuardrailValue::SignedInteger(3),
        )]),
        extra_headers: BTreeMap::new(),
        score_threshold: None,
        block_on_detection: true,
        ssl_verification: Verification::Enabled,
        timeout: None,
    }
}

#[tokio::test]
async fn client_batches_texts_in_one_request() {
    let (base, mut captured, mut server) = capturing_server(
        "/api/v1/text/contents",
        StatusCode::OK,
        r#"[[{"detection_type":"toxic","score":0.9,"text":"bad"}],[]]"#.to_owned(),
        4,
    )
    .await;
    let client = IbmDetectorClient::new(config(true, &format!("{base}/api/v1/text/contents")))
        .expect("client");
    let detections = client
        .call_detector_server(&["first".to_owned(), "second".to_owned()])
        .await
        .expect("call");
    assert_eq!(detections.len(), 2);
    assert_eq!(detections[0].len(), 1);
    assert_eq!(detections[0][0].detection_type, "toxic");
    let request = captured.recv().await.expect("request");
    let body: DetectorServerBody = serde_json::from_str(&request.body).expect("body");
    assert_eq!(body.contents, ["first".to_owned(), "second".to_owned()]);
    assert_eq!(
        body.detector_params["threshold"],
        GuardrailValue::SignedInteger(3)
    );
    assert_eq!(
        request.headers["authorization"].to_str().unwrap(),
        "Bearer token"
    );
    assert_eq!(
        request.headers["content-type"].to_str().unwrap(),
        "application/json"
    );
    assert_eq!(
        request.headers["detector-id"].to_str().unwrap(),
        "jailbreak"
    );
    assert!(captured.try_recv().is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn client_sends_the_orchestrator_payload() {
    let (base, mut captured, mut server) = capturing_server(
        "/api/v2/text/detection/content",
        StatusCode::OK,
        r#"{"detections":[{"detection_type":"pii","score":0.5,"text":"email","detector_id":"email"}]}"#
            .to_owned(),
        4,
    )
    .await;
    let client = IbmDetectorClient::new(config(
        false,
        &format!("{base}/api/v2/text/detection/content"),
    ))
    .expect("client");
    let detections = client.call_orchestrator("hello").await.expect("call");
    assert_eq!(detections.len(), 1);
    assert_eq!(detections[0].detector_id.as_deref(), Some("email"));
    let request = captured.recv().await.expect("request");
    let body: OrchestratorBody = serde_json::from_str(&request.body).expect("body");
    assert_eq!(body.content, "hello");
    assert_eq!(
        body.detectors["jailbreak"]["threshold"],
        GuardrailValue::SignedInteger(3)
    );
    assert!(
        request.headers.get("detector-id").is_none(),
        "the orchestrator call sends no detector-id header"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn client_scans_each_text_for_the_orchestrator() {
    let (base, mut captured, mut server) = capturing_server(
        "/api/v2/text/detection/content",
        StatusCode::OK,
        r#"{"detections":[]}"#.to_owned(),
        4,
    )
    .await;
    let client = IbmDetectorClient::new(config(
        false,
        &format!("{base}/api/v2/text/detection/content"),
    ))
    .expect("client");
    let per_text = client
        .scan(&["one".to_owned(), "two".to_owned()])
        .await
        .expect("scan");
    assert_eq!(per_text, vec![Vec::new(), Vec::new()]);
    let first: OrchestratorBody =
        serde_json::from_str(&captured.recv().await.expect("first request").body).expect("body");
    assert_eq!(first.content, "one");
    let second: OrchestratorBody =
        serde_json::from_str(&captured.recv().await.expect("second request").body).expect("body");
    assert_eq!(second.content, "two");
    server.shutdown().await;
}

#[tokio::test]
async fn client_scans_in_one_request_for_the_detector_server() {
    let (base, mut captured, mut server) = capturing_server(
        "/api/v1/text/contents",
        StatusCode::OK,
        "[[],[]]".to_owned(),
        4,
    )
    .await;
    let client = IbmDetectorClient::new(config(true, &format!("{base}/api/v1/text/contents")))
        .expect("client");
    let per_text = client
        .scan(&["one".to_owned(), "two".to_owned()])
        .await
        .expect("scan");
    assert_eq!(per_text, vec![Vec::new(), Vec::new()]);
    let request = captured.recv().await.expect("request");
    let body: DetectorServerBody = serde_json::from_str(&request.body).expect("body");
    assert_eq!(body.contents, ["one".to_owned(), "two".to_owned()]);
    assert!(captured.try_recv().is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn client_reports_an_error_status() {
    let (base, _captured, mut server) = capturing_server(
        "/api/v1/text/contents",
        StatusCode::INTERNAL_SERVER_ERROR,
        "detector unavailable".to_owned(),
        4,
    )
    .await;
    let client = IbmDetectorClient::new(config(true, &format!("{base}/api/v1/text/contents")))
        .expect("client");
    let error = client
        .call_detector_server(&["hello".to_owned()])
        .await
        .expect_err("error status");
    assert!(matches!(
        error,
        IbmClientError::Status {
            status: StatusCode::INTERNAL_SERVER_ERROR
        }
    ));
    server.shutdown().await;
}

#[tokio::test]
async fn client_rejects_a_malformed_response() {
    let (base, _captured, mut server) = capturing_server(
        "/api/v1/text/contents",
        StatusCode::OK,
        r#"{"detections":[]}"#.to_owned(),
        4,
    )
    .await;
    let client = IbmDetectorClient::new(config(true, &format!("{base}/api/v1/text/contents")))
        .expect("client");
    let error = client
        .call_detector_server(&["hello".to_owned()])
        .await
        .expect_err("malformed response");
    assert!(matches!(error, IbmClientError::ResponseJson { .. }));
    server.shutdown().await;
}

#[tokio::test]
async fn client_reads_an_orchestrator_answer_without_detections() {
    let (base, _captured, mut server) = capturing_server(
        "/api/v2/text/detection/content",
        StatusCode::OK,
        "{}".to_owned(),
        4,
    )
    .await;
    let client = IbmDetectorClient::new(config(
        false,
        &format!("{base}/api/v2/text/detection/content"),
    ))
    .expect("client");
    let detections = client.call_orchestrator("hello").await.expect("call");
    assert!(detections.is_empty());
    server.shutdown().await;
}

fn server_config() -> Arc<ServerConfig> {
    let certificates = CertificateDer::pem_slice_iter(SERVER_CERT)
        .collect::<Result<Vec<_>, _>>()
        .expect("certificates");
    let private_key = PrivateKeyDer::from_pem_slice(SERVER_KEY).expect("private key");
    let config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)
        .expect("server config");
    Arc::new(config)
}

fn accept(listener: &StdTcpListener) -> io::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let started = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _address)) => return Ok(stream),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if started.elapsed() >= TLS_TIMEOUT {
                    return Err(io::Error::new(
                        ErrorKind::TimedOut,
                        "client did not connect",
                    ));
                }
                thread::park_timeout(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

fn serve_tls_once(listener: &StdTcpListener, body: &[u8]) -> io::Result<()> {
    let mut stream = accept(listener)?;
    stream.set_read_timeout(Some(TLS_TIMEOUT))?;
    stream.set_write_timeout(Some(TLS_TIMEOUT))?;
    let mut acceptor = Acceptor::default();
    let accepted = loop {
        if acceptor.read_tls(&mut stream)? == 0 {
            return Err(io::Error::from(ErrorKind::UnexpectedEof));
        }
        if let Some(accepted) = acceptor
            .accept()
            .map_err(|(error, _alert)| io::Error::other(error))?
        {
            break accepted;
        }
    };
    let connection = accepted
        .into_connection(server_config())
        .map_err(|(error, _alert)| io::Error::other(error))?;
    let mut tls = StreamOwned::new(connection, stream);
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
        let mut byte = [0_u8];
        tls.read_exact(&mut byte)?;
        request.extend_from_slice(&byte);
    }
    write!(
        tls,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )?;
    tls.write_all(body)?;
    tls.flush()
}

fn run_tls_case(verification: Verification, succeeds: bool) {
    thread::scope(|scope| {
        let listener = StdTcpListener::bind(("127.0.0.1", 0)).expect("listener");
        let address = listener.local_addr().expect("address");
        let server = scope.spawn(move || serve_tls_once(&listener, b"[[]]"));
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut settings = config(
            true,
            &format!("https://127.0.0.1:{}/api/v1/text/contents", address.port()),
        );
        settings.ssl_verification = verification;
        settings.timeout = Some(TLS_TIMEOUT);
        let client = IbmDetectorClient::new(settings).expect("client");
        let result = runtime.block_on(client.call_detector_server(&["hello".to_owned()]));
        assert_eq!(result.is_ok(), succeeds, "{result:?}");
        if !succeeds {
            assert!(matches!(result, Err(IbmClientError::Request(_))));
        }
        drop(server.join());
    });
}

#[test]
fn client_verifies_the_detector_certificate() {
    run_tls_case(Verification::Enabled, false);
}

#[test]
fn client_skips_certificate_verification_when_disabled() {
    run_tls_case(Verification::Disabled, true);
}
