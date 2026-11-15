use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use betterlite_config::tls::Verification;
use rstest::rstest;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, ServerConfig, ServerConnection};

use crate::tls::{TlsError, websocket_client_config};

const TIMEOUT: Duration = Duration::from_secs(5);

struct LocalServer {
    address: SocketAddr,
    ca_pem: String,
    listener: TcpListener,
    config: Arc<ServerConfig>,
}

fn local_server() -> LocalServer {
    let certified =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("certificate");
    let provider = Arc::new(aws_lc_rs::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certified.cert.der().to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                certified.signing_key.serialize_der(),
            )),
        )
        .expect("server config");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    LocalServer {
        address,
        ca_pem: certified.cert.pem(),
        listener,
        config: Arc::new(config),
    }
}

fn serve(listener: TcpListener, config: Arc<ServerConfig>) {
    let (mut stream, _peer) = listener.accept().expect("accept");
    stream
        .set_read_timeout(Some(TIMEOUT))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(TIMEOUT))
        .expect("write timeout");
    let mut connection = ServerConnection::new(config).expect("server connection");
    // a client that rejects the certificate reports it through its own error, and its alert ends this flight
    let _handshake = connection.complete_io(&mut stream);
}

fn handshake(config: ClientConfig, address: SocketAddr) -> bool {
    let name = ServerName::try_from("localhost").expect("server name");
    let mut connection = ClientConnection::new(Arc::new(config), name).expect("client connection");
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).expect("connect");
    stream
        .set_read_timeout(Some(TIMEOUT))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(TIMEOUT))
        .expect("write timeout");
    connection.complete_io(&mut stream).is_ok() && !connection.is_handshaking()
}

#[rstest]
#[case::verification_disabled(Verification::Disabled, true)]
#[case::bundled_roots_reject_a_self_signed_server(Verification::Enabled, false)]
fn websocket_policy_tests_behavior(#[case] verification: Verification, #[case] trusted: bool) {
    let LocalServer {
        address,
        listener,
        config,
        ..
    } = local_server();
    thread::scope(|scope| {
        scope.spawn(move || serve(listener, config));
        let client = websocket_client_config(&verification).expect("client config");
        assert_eq!(handshake(client, address), trusted);
    });
}

#[test]
fn websocket_config_trusts_the_configured_ca_file() {
    let LocalServer {
        address,
        ca_pem,
        listener,
        config,
    } = local_server();
    let ca_file = tempfile::NamedTempFile::new().expect("ca file");
    fs::write(ca_file.path(), ca_pem).expect("ca pem");
    let path = ca_file.path().to_str().expect("path").to_owned();
    thread::scope(|scope| {
        scope.spawn(move || serve(listener, config));
        let client = websocket_client_config(&Verification::CaFile(path)).expect("client config");
        assert!(handshake(client, address));
    });
}

#[test]
fn websocket_config_reports_a_ca_file_it_cannot_read() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("missing-ca.pem");
    let path = path.to_str().expect("path").to_owned();
    let result = websocket_client_config(&Verification::CaFile(path));
    assert!(matches!(result, Err(TlsError::ReadCa(_))));
}

#[test]
fn websocket_config_reports_a_ca_file_without_certificates() {
    let ca_file = tempfile::NamedTempFile::new().expect("ca file");
    fs::write(ca_file.path(), b"not a certificate").expect("ca contents");
    let path = ca_file.path().to_str().expect("path").to_owned();
    let result = websocket_client_config(&Verification::CaFile(path));
    assert!(matches!(
        result,
        Err(TlsError::EmptyCa | TlsError::InvalidPem(_))
    ));
}
