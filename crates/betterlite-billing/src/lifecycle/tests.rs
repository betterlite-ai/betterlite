#![cfg(test)]

use std::fs;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use betterlite_config::sections::billing_metrics::Settings;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
use secrecy::SecretString;

use super::{LicenseData, RecorderContext, build_recorder, shutdown_recorder};
use crate::active_recorder::ActiveRecorderRegistry;
use crate::billable_category::BillableCategory;
use crate::tests::{direct_policy, fixture, logged};

struct Received {
    request: Vec<u8>,
    client_certificate: Vec<u8>,
}

struct Metered {
    endpoint: String,
    startup: String,
    shutdown: String,
    received: Received,
    registry: ActiveRecorderRegistry,
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
        temp_root_candidates: vec![temp_root.to_path_buf()],
    }
}

fn server_config() -> Arc<ServerConfig> {
    let provider = Arc::new(aws_lc_rs::default_provider());
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

fn request_complete(buffer: &[u8]) -> bool {
    let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
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
    let mut request = Vec::new();
    let mut chunk = [0_u8; 8192];
    while !request_complete(&request) {
        let read = tls.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "the client closed the connection early");
        request.extend_from_slice(&chunk[..read]);
    }
    tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
        .unwrap();
    tls.flush().unwrap();
    let client_certificate = tls.conn.peer_certificates().unwrap()[0].to_vec();
    Received {
        request,
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

fn meter_one_request(user_id: Option<&str>, model_id: Option<&str>) -> Metered {
    let scratch = tempfile::tempdir().unwrap();
    let registry = ActiveRecorderRegistry::default();
    let (endpoint, exports, _server) = collector();
    let configured = settings(&endpoint, scratch.path());
    let license = LicenseData {
        user_id: user_id.map(str::to_owned),
    };
    let (recorder, startup) = logged(|| {
        build_recorder(
            true,
            Some(&license),
            "1.105.1",
            &RecorderContext {
                settings: &configured,
                proxy: &direct_policy(),
                registry: &registry,
            },
        )
    });
    recorder
        .unwrap()
        .record(BillableCategory::Llm, "/v1/chat/completions", 200, model_id);
    let ((), shutdown) = logged(|| shutdown_recorder(&registry));
    let received = exports.recv_timeout(Duration::from_secs(5)).unwrap();
    Metered {
        endpoint,
        startup,
        shutdown,
        received,
        registry,
    }
}

fn absent<'needle>(request: &[u8], needles: &[&'needle str]) -> Vec<&'needle str> {
    needles
        .iter()
        .copied()
        .filter(|needle| {
            !request
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
        })
        .collect()
}

#[test]
fn an_unlicensed_deployment_gets_no_recorder() {
    let scratch = tempfile::tempdir().unwrap();
    let registry = ActiveRecorderRegistry::default();
    let configured = settings("https://localhost:9", scratch.path());
    let (recorder, output) = logged(|| {
        build_recorder(
            false,
            None,
            "1.105.1",
            &RecorderContext {
                settings: &configured,
                proxy: &direct_policy(),
                registry: &registry,
            },
        )
    });
    assert!(recorder.is_none() && registry.pop().is_none());
    let disabled =
        "DEBUG betterlite_billing::lifecycle: Billing metrics disabled: deployment is not licensed";
    assert!(output.contains(disabled), "{output}");
    assert!(!output.contains("WARN"));
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
}

#[test]
fn missing_configuration_disables_metering() {
    let registry = ActiveRecorderRegistry::default();
    let (recorder, output) = logged(|| {
        build_recorder(
            true,
            None,
            "1.105.1",
            &RecorderContext {
                settings: &Settings::default(),
                proxy: &direct_policy(),
                registry: &registry,
            },
        )
    });
    assert!(recorder.is_none() && registry.pop().is_none());
    assert!(
        output.contains("(LITELLM_BILLING_METRICS_ENDPOINT, "),
        "{output}"
    );
}

#[test]
fn an_exporter_failure_disables_metering() {
    let scratch = tempfile::tempdir().unwrap();
    let registry = ActiveRecorderRegistry::default();
    let mut configured = settings("https://localhost:9", scratch.path());
    configured.export_interval_ms = 0;
    let (recorder, output) = logged(|| {
        build_recorder(
            true,
            None,
            "1.105.1",
            &RecorderContext {
                settings: &configured,
                proxy: &direct_policy(),
                registry: &registry,
            },
        )
    });
    assert!(recorder.is_none() && registry.pop().is_none());
    let disabled = "WARN betterlite_billing::lifecycle: Billing metrics disabled: failed to initialize exporter: interval value 0 is invalid";
    assert!(output.contains(disabled), "{output}");
}

#[test]
fn a_licensed_deployment_meters_over_mutual_tls() {
    let metered = meter_one_request(Some("license-7"), Some("model-a"));
    let started = format!(
        "INFO betterlite_billing::lifecycle: Billing metrics enabled: exporting to {}v1/metrics every 60000 ms",
        metered.endpoint
    );
    assert!(metered.startup.contains(&started), "{}", metered.startup);
    assert!(!metered.shutdown.contains("WARN"), "{}", metered.shutdown);
    let ((), idle) = logged(|| shutdown_recorder(&metered.registry));
    assert_eq!(idle, "");
    let client = certificate("client.crt").to_vec();
    assert_eq!(metered.received.client_certificate, client);
    let needles = [
        "POST /v1/metrics HTTP/1.1",
        "content-type: application/x-protobuf",
        "litellm.enterprise.billable_requests",
        "litellm.license.id",
        "license-7",
        "litellm-proxy",
        "1.105.1",
        "/v1/chat/completions",
        "litellm.model_id",
        "model-a",
    ];
    assert_eq!(
        absent(&metered.received.request, &needles),
        Vec::<&str>::new()
    );
}

#[test]
fn a_deployment_without_a_user_id_exports_no_license_id() {
    let request = meter_one_request(None, None).received.request;
    let missing = ["litellm.license.id", "litellm.model_id"];
    assert_eq!(absent(&request, &missing), missing);
    assert!(absent(&request, &["litellm.enterprise.billable_requests"]).is_empty());
}

#[test]
fn a_failed_final_flush_is_logged() {
    let scratch = tempfile::tempdir().unwrap();
    let registry = ActiveRecorderRegistry::default();
    let configured = settings("https://localhost:9", scratch.path());
    let recorder = build_recorder(
        true,
        None,
        "1.105.1",
        &RecorderContext {
            settings: &configured,
            proxy: &direct_policy(),
            registry: &registry,
        },
    )
    .unwrap();
    recorder.shutdown();
    let ((), output) = logged(|| shutdown_recorder(&registry));
    assert!(
        output.contains("WARN") && output.contains("final flush failed"),
        "{output}"
    );
    assert!(registry.pop().is_none());
}
