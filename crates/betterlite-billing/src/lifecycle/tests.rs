#![cfg(test)]

use std::fs;
use std::io::{self, Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use betterlite_config::sections::billing_metrics::Settings;
use rustls::crypto::ring;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
use secrecy::SecretString;
use tracing::Level;
use tracing::subscriber::with_default;

use super::{LicenseData, build_recorder, shutdown_recorder};
use crate::active_recorder::ActiveRecorderRegistry;
use crate::billable_category::BillableCategory;

struct Received {
    head: String,
    body: Vec<u8>,
    client_certificate: Vec<u8>,
}

struct Metered {
    endpoint: String,
    startup: String,
    shutdown: String,
    request: Received,
    registry: ActiveRecorderRegistry,
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let result = with_default(subscriber, action);
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    (result, output)
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn certificate(name: &str) -> CertificateDer<'static> {
    CertificateDer::from_pem_file(fixture(name)).unwrap()
}

fn settings(endpoint: &str, temp_root: &Path) -> Settings {
    Settings {
        endpoint: Some(endpoint.to_owned()),
        client_cert: Some(fs::read_to_string(fixture("client.crt")).unwrap()),
        client_key: Some(SecretString::from(fixture("client.key").to_str().unwrap())),
        ca_cert: Some(fixture("ca.crt").to_str().unwrap().to_owned()),
        export_interval_ms: 60_000,
        temp_root: temp_root.to_path_buf(),
    }
}

fn server_config() -> Arc<ServerConfig> {
    let provider = Arc::new(ring::default_provider());
    let mut roots = RootCertStore::empty();
    roots.add(certificate("ca.crt")).unwrap();
    let verifier =
        WebPkiClientVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
            .build()
            .unwrap();
    let key = PrivateKeyDer::from_pem_file(fixture("server.key")).unwrap();
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![certificate("server.crt")], key)
        .unwrap();
    Arc::new(config)
}

fn head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn request_complete(buffer: &[u8]) -> bool {
    let Some(end) = head_end(buffer) else {
        return false;
    };
    let head = str::from_utf8(&buffer[..end]).unwrap().to_ascii_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .map_or(0, |value| value.trim().parse::<usize>().unwrap());
    buffer.len() >= end.checked_add(4).unwrap().checked_add(length).unwrap()
}

fn serve_one(listener: &TcpListener) -> Received {
    let (stream, _peer) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut tls = StreamOwned::new(ServerConnection::new(server_config()).unwrap(), stream);
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    while !request_complete(&buffer) {
        let read = tls.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "the client closed the connection early");
        buffer.extend_from_slice(&chunk[..read]);
    }
    tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
        .unwrap();
    tls.flush().unwrap();
    let end = head_end(&buffer).unwrap();
    let client_certificate = tls.conn.peer_certificates().unwrap()[0].to_vec();
    Received {
        head: str::from_utf8(&buffer[..end]).unwrap().to_owned(),
        body: buffer[end.checked_add(4).unwrap()..].to_vec(),
        client_certificate,
    }
}

fn collector() -> (String, Receiver<Received>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, receiver) = mpsc::sync_channel(1);
    let server = thread::Builder::new()
        .name("mtls-collector".to_owned())
        .spawn(move || sender.send(serve_one(&listener)).unwrap())
        .unwrap();
    (format!("https://localhost:{port}/"), receiver, server)
}

fn meter_one_request(license: &LicenseData, model_id: Option<&str>) -> Metered {
    let scratch = tempfile::tempdir().unwrap();
    let registry = ActiveRecorderRegistry::default();
    let (endpoint, received, _server) = collector();
    let configured = settings(&endpoint, scratch.path());
    let (recorder, startup) =
        logged(|| build_recorder(true, Some(license), "1.105.1", &configured, &registry));
    recorder
        .unwrap()
        .record(BillableCategory::Llm, "/v1/chat/completions", 200, model_id);
    let ((), shutdown) = logged(|| shutdown_recorder(&registry));
    let request = received.recv_timeout(Duration::from_secs(20)).unwrap();
    Metered {
        endpoint,
        startup,
        shutdown,
        request,
        registry,
    }
}

fn contains(body: &[u8], needle: &str) -> bool {
    body.windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn licensed() -> LicenseData {
    LicenseData {
        user_id: Some("license-7".to_owned()),
    }
}

#[test]
fn an_unlicensed_deployment_gets_no_recorder() {
    let scratch = tempfile::tempdir().unwrap();
    let registry = ActiveRecorderRegistry::default();
    let configured = settings("https://localhost:9", scratch.path());
    let (recorder, output) =
        logged(|| build_recorder(false, None, "1.105.1", &configured, &registry));
    assert!(recorder.is_none());
    assert!(registry.pop().is_none());
    assert!(output.contains("DEBUG"), "{output}");
    assert!(
        output.contains("Billing metrics disabled: deployment is not licensed"),
        "{output}"
    );
    assert!(!output.contains("WARN"));
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
}

#[test]
fn missing_configuration_disables_metering() {
    let registry = ActiveRecorderRegistry::default();
    let (recorder, output) =
        logged(|| build_recorder(true, None, "1.105.1", &Settings::default(), &registry));
    assert!(recorder.is_none());
    assert!(registry.pop().is_none());
    assert!(
        output.contains("LITELLM_BILLING_METRICS_ENDPOINT"),
        "{output}"
    );
}

#[test]
fn an_exporter_failure_disables_metering() {
    let scratch = tempfile::tempdir().unwrap();
    let registry = ActiveRecorderRegistry::default();
    let mut configured = settings("https://localhost:9", scratch.path());
    configured.export_interval_ms = 0;
    let (recorder, output) =
        logged(|| build_recorder(true, None, "1.105.1", &configured, &registry));
    assert!(recorder.is_none());
    assert!(registry.pop().is_none());
    assert!(output.contains("WARN"), "{output}");
    assert!(
        output.contains(
            "Billing metrics disabled: failed to initialize exporter: interval value 0 is invalid"
        ),
        "{output}"
    );
}

#[test]
fn a_licensed_deployment_logs_where_it_exports() {
    let metered = meter_one_request(&licensed(), Some("model-a"));
    assert!(metered.startup.contains("INFO"), "{}", metered.startup);
    let started = format!(
        "Billing metrics enabled: exporting to {}v1/metrics every 60000 ms",
        metered.endpoint
    );
    assert!(metered.startup.contains(&started), "{}", metered.startup);
    assert!(!metered.shutdown.contains("WARN"), "{}", metered.shutdown);
    let ((), idle) = logged(|| shutdown_recorder(&metered.registry));
    assert_eq!(idle, "");
}

#[test]
fn the_collector_sees_the_client_certificate() {
    let request = meter_one_request(&licensed(), Some("model-a")).request;
    assert!(
        request.head.starts_with("POST /v1/metrics HTTP/1.1"),
        "{}",
        request.head
    );
    assert!(
        request
            .head
            .to_ascii_lowercase()
            .contains("content-type: application/x-protobuf")
    );
    assert_eq!(
        request.client_certificate,
        certificate("client.crt").to_vec()
    );
}

#[test]
fn the_export_names_the_deployment_and_the_request() {
    let request = meter_one_request(&licensed(), Some("model-a")).request;
    let absent: Vec<&str> = [
        "litellm.enterprise.billable_requests",
        "litellm.license.id",
        "license-7",
        "litellm-proxy",
        "1.105.1",
        "/v1/chat/completions",
        "litellm.model_id",
        "model-a",
    ]
    .into_iter()
    .filter(|needle| !contains(&request.body, needle))
    .collect();
    assert!(absent.is_empty(), "missing from the export: {absent:?}");
}

#[test]
fn a_deployment_without_a_user_id_exports_no_license_id() {
    let request = meter_one_request(&LicenseData { user_id: None }, None).request;
    assert!(contains(
        &request.body,
        "litellm.enterprise.billable_requests"
    ));
    assert!(!contains(&request.body, "litellm.license.id"));
    assert!(!contains(&request.body, "litellm.model_id"));
}

#[test]
fn a_failed_final_flush_is_logged() {
    let scratch = tempfile::tempdir().unwrap();
    let registry = ActiveRecorderRegistry::default();
    let configured = settings("https://localhost:9", scratch.path());
    let recorder = build_recorder(true, None, "1.105.1", &configured, &registry).unwrap();
    recorder.shutdown();
    registry.set(recorder);
    let ((), output) = logged(|| shutdown_recorder(&registry));
    assert!(output.contains("WARN"), "{output}");
    assert!(
        output.contains("Billing metrics: final flush failed"),
        "{output}"
    );
    assert!(registry.pop().is_none());
}
