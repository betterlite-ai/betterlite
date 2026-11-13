use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use url::Url;

// SSLRequest is an i32 length of 8 and the i32 code 80877103, both big-endian
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 4, 210, 22, 47];

pub(super) enum Mode {
    Tls {
        leaf: CertificateDer<'static>,
        key: PrivateKeyDer<'static>,
    },
    Plaintext,
}

pub(super) struct Front {
    pub(super) port: u16,
    handshakes: Arc<AtomicUsize>,
    tasks: JoinSet<()>,
}

impl Front {
    pub(super) async fn start(upstream_url: &str, mode: Mode) -> Self {
        let parsed = Url::parse(upstream_url).expect("the upstream server URL");
        let host = parsed.host_str().expect("an upstream host").to_owned();
        let port = parsed.port().unwrap_or(5432);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("a front listener");
        let front_port = listener.local_addr().expect("a bound front address").port();
        let acceptor = match &mode {
            Mode::Tls { leaf, key } => {
                let config = ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(vec![leaf.clone()], key.clone_key())
                    .expect("a front TLS config");
                Some(TlsAcceptor::from(Arc::new(config)))
            }
            Mode::Plaintext => None,
        };
        let handshakes = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();
        tasks.spawn(accept(
            listener,
            acceptor,
            Target { host, port },
            Arc::clone(&handshakes),
        ));
        Self {
            port: front_port,
            handshakes,
            tasks,
        }
    }

    pub(super) fn handshakes(&self) -> usize {
        self.handshakes.load(Ordering::SeqCst)
    }
}

impl Drop for Front {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

#[derive(Clone)]
struct Target {
    host: String,
    port: u16,
}

async fn accept(
    listener: TcpListener,
    acceptor: Option<TlsAcceptor>,
    target: Target,
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
    acceptor: Option<TlsAcceptor>,
    target: Target,
    handshakes: Arc<AtomicUsize>,
) {
    if !ssl_request(&mut socket).await {
        return;
    }
    let Some(tls_acceptor) = acceptor else {
        if socket.write_all(b"N").await.is_err() {
            return;
        }
        proxy(&mut socket, &target).await;
        return;
    };
    if socket.write_all(b"S").await.is_err() {
        return;
    }
    handshakes.fetch_add(1, Ordering::SeqCst);
    let Ok(mut client) = tls_acceptor.accept(socket).await else {
        return;
    };
    proxy(&mut client, &target).await;
}

async fn proxy<S>(client: &mut S, target: &Target)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let Ok(mut server) = TcpStream::connect((target.host.as_str(), target.port)).await else {
        return;
    };
    drop(copy_bidirectional(client, &mut server).await);
}

async fn ssl_request(socket: &mut TcpStream) -> bool {
    let mut request = [0_u8; SSL_REQUEST.len()];
    if socket.read_exact(&mut request).await.is_err() {
        return false;
    }
    request == SSL_REQUEST
}
