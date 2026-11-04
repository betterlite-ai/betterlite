#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Error as IoError, ErrorKind, Read as _, Write as _, copy};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::ProxyTrust;
use rstest::rstest;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{Acceptor, WebPkiClientVerifier};
use rustls::{RootCertStore, ServerConfig, StreamOwned};
use tokio::runtime::Builder;

use crate::proxy::ProxiedClient;

const SERVER_CERT: &[u8] =
    include_bytes!("../../../../../tools/postgres-fixture/fixtures/tls/server.pem");
const SERVER_KEY: &[u8] =
    include_bytes!("../../../../../tools/postgres-fixture/fixtures/tls/server.key");
const TEST_CA: &[u8] = include_bytes!("../../../../../tools/postgres-fixture/fixtures/tls/ca.pem");
const TIMEOUT: Duration = Duration::from_secs(5);

#[rstest]
#[case::trusted_ca("", "localhost", true)]
#[case::untrusted_ca("untrusted", "localhost", false)]
#[case::wrong_host("", "127.0.0.1", false)]
#[case::verification_disabled("disabled", "127.0.0.1", true)]
#[case::deployment_over_environment("deployment", "127.0.0.1", true)]
#[case::disabled_with_curve("disabled_curve", "127.0.0.1", true)]
#[case::trusted_with_curve("curve", "localhost", true)]
#[case::certificate_does_not_enable_mtls("certificate", "localhost", true)]
#[case::http2_enabled("http2", "localhost", true)]
#[case::http2_with_curve("http2_curve", "localhost", true)]
#[case::empty_user_agent("empty_agent", "localhost", true)]
#[case::user_agent_override("agent", "localhost", true)]
#[case::https_connect_auth("proxy", "localhost", true)]
#[case::https_connect_rejects_wrong_host("proxy", "127.0.0.1", false)]
fn http_policy_tests_behavior(#[case] mode: &str, #[case] host: &str, #[case] succeeds: bool) {
    thread::scope(|scope| {
        let ca_file = tempfile::NamedTempFile::new().unwrap();
        fs::write(ca_file.path(), TEST_CA).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = scope.spawn(move || serve(&listener));
        let mut environment = environment(mode, ca_file.path().to_str().unwrap());
        let proxy = (mode == "proxy").then(|| {
            let proxy_listener = TcpListener::bind("127.0.0.1:0").unwrap();
            environment.insert(
                "HTTPS_PROXY".to_owned(),
                format!(
                    "http://user:secret@{}",
                    proxy_listener.local_addr().unwrap()
                ),
            );
            scope.spawn(move || tunnel(&proxy_listener, address))
        });
        let yaml = if mode == "deployment" {
            "model_list:\n  - model_name: test\n    litellm_params:\n      model: provider/model\n      ssl_verify: false\n"
        } else {
            ""
        };
        let config = ConfigLoader {
            lookup: BTreeMap::new(),
        }
        .load(yaml)
        .unwrap();
        let policy = TlsPolicy::resolve(&config, &environment, None).unwrap();
        let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
        let client = config.model_list.first().map_or_else(
            || ProxiedClient::outbound_with_tls(&routing, &policy).unwrap(),
            |deployment| {
                ProxiedClient::outbound_for_deployment(&routing, &config, deployment, &environment)
                    .unwrap()
            },
        );
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let _entered = runtime.enter();
        let response = runtime.block_on(
            client
                .client
                .get(format!("https://{host}:{}", address.port()))
                .timeout(TIMEOUT)
                .send(),
        );
        assert_eq!(response.is_ok(), succeeds, "{response:?}");
        let observation = server.join().unwrap();
        if succeeds {
            let (request, offered_h2, sent_identity) = observation.unwrap();
            assert!(!sent_identity);
            let expected_agent = match mode {
                "agent" => "agent/1".to_owned(),
                "empty_agent" => String::new(),
                _ => format!("betterlite/{}", env!("CARGO_PKG_VERSION")),
            };
            assert_eq!(policy.user_agent, expected_agent);
            assert!(request.contains(&format!("user-agent: {}\r\n", policy.user_agent)));
            assert!(!request.contains("proxy-authorization"));
            assert_eq!(offered_h2, mode.starts_with("http2"));
        } else {
            let _error = observation.unwrap_err();
        }
        if let Some(proxy_thread) = proxy {
            proxy_thread.join().unwrap();
        }
    });
}

fn environment(mode: &str, ca_path: &str) -> BTreeMap<String, String> {
    let mut variables = BTreeMap::new();
    if !matches!(mode, "untrusted" | "disabled" | "disabled_curve") {
        variables.insert("SSL_CERT_FILE".to_owned(), ca_path.to_owned());
    }
    if mode.starts_with("disabled") {
        variables.insert("SSL_VERIFY".to_owned(), "false".to_owned());
    }
    if mode.ends_with("curve") {
        variables.insert("SSL_ECDH_CURVE".to_owned(), "X25519".to_owned());
    }
    let additional = match mode {
        "certificate" => Some(("SSL_CERTIFICATE", "not-read-client.pem")),
        "deployment" => Some(("SSL_VERIFY", "true")),
        "http2" | "http2_curve" => Some(("LITELLM_HTTP2", "true")),
        "empty_agent" => Some(("LITELLM_USER_AGENT", "")),
        "agent" => Some(("LITELLM_USER_AGENT", "agent/1")),
        _ => None,
    };
    if let Some((name, value)) = additional {
        variables.insert(name.to_owned(), value.to_owned());
    }
    variables
}

fn serve(listener: &TcpListener) -> Result<(String, bool, bool), IoError> {
    let mut stream = accept(listener);
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut acceptor = Acceptor::default();
    let accepted = loop {
        if acceptor.read_tls(&mut stream)? == 0 {
            return Err(IoError::from(ErrorKind::UnexpectedEof));
        }
        if let Some(accepted) = acceptor
            .accept()
            .map_err(|(error, _alert)| IoError::other(error))?
        {
            break accepted;
        }
    };
    let offered_h2 = accepted
        .client_hello()
        .alpn()
        .is_some_and(|mut protocols| protocols.any(|protocol| protocol == b"h2"));
    let connection = accepted
        .into_connection(server_config())
        .map_err(|(error, _alert)| IoError::other(error))?;
    let mut tls_stream = StreamOwned::new(connection, stream);
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
        let mut byte = [0];
        tls_stream.read_exact(&mut byte)?;
        request.extend_from_slice(&byte);
    }
    let sent_identity = tls_stream.conn.peer_certificates().is_some();
    tls_stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
    tls_stream.flush()?;
    let request_text = String::from_utf8(request).map_err(IoError::other)?;
    Ok((request_text, offered_h2, sent_identity))
}

fn server_config() -> Arc<ServerConfig> {
    let certificates = CertificateDer::pem_slice_iter(SERVER_CERT)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let private_key = PrivateKeyDer::from_pem_slice(SERVER_KEY).unwrap();
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(TEST_CA).unwrap())
        .unwrap();
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .allow_unauthenticated()
        .build()
        .unwrap();
    let config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certificates, private_key)
        .unwrap();
    Arc::new(config)
}

fn tunnel(listener: &TcpListener, origin: SocketAddr) {
    let mut downstream = accept(listener);
    downstream.set_read_timeout(Some(TIMEOUT)).unwrap();
    downstream.set_write_timeout(Some(TIMEOUT)).unwrap();
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
        let mut byte = [0];
        downstream.read_exact(&mut byte).unwrap();
        request.extend_from_slice(&byte);
    }
    let text = String::from_utf8(request).unwrap();
    assert!(text.starts_with("CONNECT "));
    assert!(
        text.to_ascii_lowercase()
            .contains("proxy-authorization: basic dxnlcjpzzwnyzxq=")
    );
    let mut upstream = TcpStream::connect_timeout(&origin, TIMEOUT).unwrap();
    upstream.set_read_timeout(Some(TIMEOUT)).unwrap();
    upstream.set_write_timeout(Some(TIMEOUT)).unwrap();
    downstream
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .unwrap();
    let mut downstream_reader = downstream.try_clone().unwrap();
    let mut upstream_writer = upstream.try_clone().unwrap();
    thread::scope(|scope| {
        scope.spawn(move || {
            let _copied = copy(&mut downstream_reader, &mut upstream_writer);
            let _shutdown = upstream_writer.shutdown(Shutdown::Write);
        });
        let _copied = copy(&mut upstream, &mut downstream);
        let _shutdown = downstream.shutdown(Shutdown::Both);
    });
}

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let started = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) => assert_eq!(error.kind(), ErrorKind::WouldBlock),
        }
        assert!(started.elapsed() < TIMEOUT, "client did not connect");
        thread::park_timeout(Duration::from_millis(10));
    }
}
