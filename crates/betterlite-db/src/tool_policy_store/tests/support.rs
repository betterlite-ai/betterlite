use std::io;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinSet, yield_now};
use tokio_postgres::NoTls;
use tokio_postgres_rustls_improved::MakeRustlsConnect;
use tokio_rustls::TlsAcceptor;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;
use url::Url;

use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::postgres_store::session::PostgresStore;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Db {
    pub(crate) fixture: PostgresFixture,
    pub(crate) store: PostgresStore,
}

impl Db {
    pub(crate) async fn start() -> Self {
        let url = Environment::parse_from(["tool-policy-store"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(
            url.as_ref(),
            catalog_statements(&PACKAGED_CATALOGS).unwrap(),
        )
        .resolve()
        .await
        .unwrap();
        let config = fixture.options.url.expose_secret().parse().unwrap();
        let manager = Manager::new(config, NoTls);
        let pool = Pool::builder(manager).max_size(2).build().unwrap();
        Self {
            fixture,
            store: PostgresStore { pool: Some(pool) },
        }
    }

    pub(crate) async fn client(&self) -> Client {
        self.store.session().await.unwrap()
    }

    pub(crate) async fn batch(&self, sql: &str) {
        let client = self.client().await;
        client.batch_execute(sql).await.unwrap();
    }

    pub(crate) async fn close(self) {
        self.store.pool.unwrap().close();
        // The closed pool's drivers need a turn to close their sockets before DROP DATABASE.
        yield_now().await;
        self.fixture.close_async().await.unwrap();
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn captured_logs() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let guard = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .set_default();
    (logs, guard)
}

// SSLRequest is an i32 length of 8 and the i32 code 80877103, both big-endian
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 4, 210, 22, 47];

type Authority = (
    CertificateDer<'static>,
    CertificateDer<'static>,
    PrivateKeyDer<'static>,
);

fn authority() -> Authority {
    let ca_key = KeyPair::generate().expect("a CA key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca_certificate = ca_params.self_signed(&ca_key).expect("a CA certificate");
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let leaf_key = KeyPair::generate().expect("a leaf key");
    let mut leaf_params =
        CertificateParams::new(vec!["localhost".to_owned()]).expect("leaf parameters");
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_certificate = leaf_params
        .signed_by(&leaf_key, &issuer)
        .expect("a leaf certificate");
    (
        ca_certificate.der().clone(),
        leaf_certificate.der().clone(),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    )
}

fn client_config(ca: CertificateDer<'static>) -> ClientConfig {
    let mut roots = RootCertStore::empty();
    roots.add(ca).expect("a trusted CA");
    ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

fn tls_config(server_url: &str, port: u16) -> tokio_postgres::Config {
    let mut parsed = Url::parse(server_url).expect("the plain server URL");
    parsed.set_host(Some("localhost")).expect("a TLS host");
    parsed.set_port(Some(port)).expect("a TLS port");
    let parameters = parsed
        .query_pairs()
        .filter_map(|(name, value)| {
            if name == "sslrootcert" || name == "sslmode" {
                return None;
            }
            Some((name.into_owned(), value.into_owned()))
        })
        .collect::<Vec<_>>();
    parsed
        .query_pairs_mut()
        .clear()
        .extend_pairs(parameters)
        .append_pair("sslmode", "require");
    parsed.as_str().parse().expect("a TLS database config")
}

// A TLS front for the plaintext fixture: it answers SSLRequest so sslmode=require sessions and their cancels speak TLS.
pub(crate) struct TlsFront {
    port: u16,
    ca: CertificateDer<'static>,
    handshakes: Arc<AtomicUsize>,
    tasks: JoinSet<()>,
}

impl TlsFront {
    pub(crate) async fn start(server_url: &str) -> Self {
        let parsed = Url::parse(server_url).expect("the plain server URL");
        let host = parsed.host_str().expect("an upstream host").to_owned();
        let server_port = parsed.port().unwrap_or(5432);
        let (ca, leaf, key) = authority();
        let config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("the default protocol versions")
            .with_no_client_auth()
            .with_single_cert(vec![leaf], key)
            .expect("a front TLS config");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("a front listener");
        let port = listener.local_addr().expect("a bound front address").port();
        let handshakes = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();
        tasks.spawn(accept_connections(
            listener,
            TlsAcceptor::from(Arc::new(config)),
            Target {
                host,
                port: server_port,
            },
            Arc::clone(&handshakes),
        ));
        Self {
            port,
            ca,
            handshakes,
            tasks,
        }
    }

    pub(crate) fn handshakes(&self) -> usize {
        self.handshakes.load(Ordering::SeqCst)
    }

    pub(crate) fn store(&self, server_url: &str) -> PostgresStore {
        let manager = Manager::new(tls_config(server_url, self.port), self.trusted_connector());
        let pool = Pool::builder(manager)
            .max_size(1)
            .build()
            .expect("a TLS pool");
        PostgresStore { pool: Some(pool) }
    }

    pub(crate) fn trusted_connector(&self) -> MakeRustlsConnect {
        MakeRustlsConnect::new(client_config(self.ca.clone()))
    }

    pub(crate) fn untrusted_connector() -> MakeRustlsConnect {
        MakeRustlsConnect::new(client_config(authority().0))
    }
}

impl Drop for TlsFront {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

#[derive(Clone)]
struct Target {
    host: String,
    port: u16,
}

async fn accept_connections(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    target: Target,
    handshakes: Arc<AtomicUsize>,
) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            break;
        };
        connections.spawn(serve_tls(
            socket,
            acceptor.clone(),
            target.clone(),
            Arc::clone(&handshakes),
        ));
        while connections.try_join_next().is_some() {}
    }
}

async fn serve_tls(
    mut socket: TcpStream,
    acceptor: TlsAcceptor,
    target: Target,
    handshakes: Arc<AtomicUsize>,
) {
    let mut request = [0_u8; SSL_REQUEST.len()];
    if socket.read_exact(&mut request).await.is_err() || request != SSL_REQUEST {
        return;
    }
    if socket.write_all(b"S").await.is_err() {
        return;
    }
    handshakes.fetch_add(1, Ordering::SeqCst);
    let Ok(mut client) = acceptor.accept(socket).await else {
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
