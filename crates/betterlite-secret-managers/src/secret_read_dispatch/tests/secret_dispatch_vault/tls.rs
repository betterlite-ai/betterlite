use std::io::Write as _;
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::ExposeSecret as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::secret_read_dispatch::http_transport::{HttpError, HttpExchange, TlsVerification};
use crate::secret_read_dispatch::reqwest_exchange::ReqwestExchange;
use crate::secret_read_dispatch::vault_http::VaultHttpTransport;
use crate::vault::transport::VaultTransport as _;

use super::socket::request;

struct Authority {
    ca_pem: String,
    server_cert: CertificateDer<'static>,
    server_key: Vec<u8>,
}

fn authority() -> Authority {
    let ca_key = KeyPair::generate().expect("a CA key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).expect("a CA certificate");
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let server_key = KeyPair::generate().expect("a server key");
    let server_params =
        CertificateParams::new(vec!["127.0.0.1".to_owned()]).expect("server parameters");
    let server_cert = server_params
        .signed_by(&server_key, &issuer)
        .expect("a server certificate");
    Authority {
        ca_pem: ca_cert.pem(),
        server_cert: server_cert.der().clone(),
        server_key: server_key.serialize_der(),
    }
}

fn ca_file(pem: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new()
        .suffix(".pem")
        .tempfile()
        .expect("a CA file");
    file.write_all(pem.as_bytes()).expect("the CA PEM");
    file
}

fn server_config(authority: &Authority) -> Arc<ServerConfig> {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(authority.server_key.clone()));
    let config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("the TLS versions")
        .with_no_client_auth()
        .with_single_cert(vec![authority.server_cert.clone()], key)
        .expect("a server config");
    Arc::new(config)
}

fn transport(tls: TlsVerification) -> VaultHttpTransport {
    let exchange: Arc<dyn HttpExchange> = Arc::new(ReqwestExchange::default());
    VaultHttpTransport { exchange, tls }
}

async fn serve(listener: &TcpListener, acceptor: &TlsAcceptor, body: &str) {
    let (socket, _) = listener.accept().await.expect("a connection");
    let mut stream = acceptor.accept(socket).await.expect("a TLS stream");
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.expect("a request byte"));
    }
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("a response");
    stream.shutdown().await.expect("a closed stream");
}

#[tokio::test]
async fn vault_transport_trusts_a_private_ca_file() {
    let server = authority();
    let ca = ca_file(&server.ca_pem);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let port = listener.local_addr().expect("an address").port();
    let acceptor = TlsAcceptor::from(server_config(&server));
    let transport = transport(TlsVerification::CaFile(ca.path().to_path_buf()));
    let url = format!("https://127.0.0.1:{port}/v1/secret/data/db");
    let (result, ()) = tokio::join!(
        transport.send(request(url)),
        serve(&listener, &acceptor, "{\"ok\":true}"),
    );
    let response = result.expect("a trusted reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "{\"ok\":true}");
}

#[tokio::test]
async fn vault_transport_rejects_a_server_outside_the_ca_file() {
    let server = authority();
    let ca = ca_file(&authority().ca_pem);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let port = listener.local_addr().expect("an address").port();
    let acceptor = TlsAcceptor::from(server_config(&server));
    let transport = transport(TlsVerification::CaFile(ca.path().to_path_buf()));
    let url = format!("https://127.0.0.1:{port}/v1/secret/data/db");
    let handshake = async {
        let (socket, _) = listener.accept().await.expect("a connection");
        drop(acceptor.accept(socket).await);
    };
    let (result, ()) = tokio::join!(transport.send(request(url)), handshake);
    assert!(matches!(result, Err(HttpError::Transport(_))));
}
