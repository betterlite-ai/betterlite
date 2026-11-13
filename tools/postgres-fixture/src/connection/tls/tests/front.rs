#![cfg(test)]

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use url::Url;

// SSLRequest is an i32 length of 8 and the i32 code 80877103, both big-endian
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 4, 210, 22, 47];
const TLS_ACCEPTED: u8 = b'S';

pub(super) struct Authority {
    pub(super) certificate_pem: String,
    leaf: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

pub(super) fn authority() -> Authority {
    let ca_key = KeyPair::generate().expect("a CA key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca_certificate = ca_params.self_signed(&ca_key).expect("a CA certificate");
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let leaf_key = KeyPair::generate().expect("a leaf key");
    // rustls matches the DNS subject alternative name and ignores the common name
    let leaf_params =
        CertificateParams::new(vec!["localhost".to_owned()]).expect("leaf parameters");
    let leaf_certificate = leaf_params
        .signed_by(&leaf_key, &issuer)
        .expect("a leaf certificate");
    Authority {
        certificate_pem: ca_certificate.pem(),
        leaf: leaf_certificate.der().clone(),
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    }
}

pub(super) struct Front {
    pub(super) port: u16,
    handshakes: Arc<AtomicUsize>,
    tasks: JoinSet<()>,
}

impl Front {
    pub(super) async fn start(server_url: &SecretString, authority: &Authority) -> Self {
        let parsed = Url::parse(server_url.expose_secret()).expect("the plain server URL");
        let host = parsed.host_str().expect("a server host").to_owned();
        let server_port = parsed.port().expect("an explicit server port");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("a front listener");
        let port = listener.local_addr().expect("a bound front address").port();
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![authority.leaf.clone()], authority.key.clone_key())
            .expect("a front TLS config");
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let handshakes = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();
        tasks.spawn(accept(
            listener,
            acceptor,
            TargetServer {
                host,
                port: server_port,
            },
            Arc::clone(&handshakes),
        ));
        Self {
            port,
            handshakes,
            tasks,
        }
    }

    pub(super) fn handshake_count(&self) -> usize {
        self.handshakes.load(Ordering::SeqCst)
    }
}

impl Drop for Front {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

#[derive(Clone)]
struct TargetServer {
    host: String,
    port: u16,
}

async fn accept(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    target: TargetServer,
    handshakes: Arc<AtomicUsize>,
) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            break;
        };
        connections.spawn(serve(
            socket,
            acceptor.clone(),
            target.clone(),
            Arc::clone(&handshakes),
        ));
        while connections.try_join_next().is_some() {}
    }
}

async fn serve(
    mut socket: TcpStream,
    acceptor: TlsAcceptor,
    target: TargetServer,
    handshakes: Arc<AtomicUsize>,
) {
    if !ssl_request(&mut socket).await {
        return;
    }
    if socket.write_all(&[TLS_ACCEPTED]).await.is_err() {
        return;
    }
    handshakes.fetch_add(1, Ordering::SeqCst);
    let Ok(mut client) = acceptor.accept(socket).await else {
        return;
    };
    let Ok(mut server) = TcpStream::connect((target.host.as_str(), target.port)).await else {
        return;
    };
    drop(copy_bidirectional(&mut client, &mut server).await);
}

async fn ssl_request(socket: &mut TcpStream) -> bool {
    let mut request = [0_u8; SSL_REQUEST.len()];
    if socket.read_exact(&mut request).await.is_err() {
        return false;
    }
    request == SSL_REQUEST
}
