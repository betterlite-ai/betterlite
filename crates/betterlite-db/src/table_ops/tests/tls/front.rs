#![cfg(test)]

use std::net::Ipv4Addr;
use std::sync::Arc;

use deadpool_postgres::{Manager, Pool};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_postgres_rustls_improved::MakeRustlsConnect;
use tokio_rustls::TlsAcceptor;
use url::Url;

pub(super) struct Front {
    pub(super) pool: Pool,
    tasks: JoinSet<()>,
}

impl Front {
    pub(super) async fn start(url: &SecretString) -> Self {
        let target = Url::parse(url.expose_secret()).unwrap();
        let host = target.host_str().unwrap().to_owned();
        let port = target.port().unwrap();
        let (server, client) = certificates();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut address = target;
        address.set_host(Some("localhost")).unwrap();
        address
            .set_port(Some(listener.local_addr().unwrap().port()))
            .unwrap();
        address.set_query(Some("sslmode=require"));
        let mut config: tokio_postgres::Config = address.as_str().parse().unwrap();
        config.application_name("table-ops-tls");
        let pool = Pool::builder(Manager::new(config, MakeRustlsConnect::new(client)))
            .max_size(1)
            .build()
            .unwrap();
        let mut tasks = JoinSet::new();
        tasks.spawn(accept(
            listener,
            TlsAcceptor::from(Arc::new(server)),
            host,
            port,
        ));
        Self { pool, tasks }
    }
}

impl Drop for Front {
    fn drop(&mut self) {
        self.pool.close();
        self.tasks.abort_all();
    }
}

fn certificates() -> (ServerConfig, ClientConfig) {
    let identity = generate_simple_self_signed(vec![String::from("localhost")]).unwrap();
    let certificate = identity.cert.der().clone();
    let key = PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
    let server = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], PrivateKeyDer::Pkcs8(key))
        .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(certificate).unwrap();
    let client = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    (server, client)
}

async fn accept(listener: TcpListener, tls: TlsAcceptor, host: String, port: u16) {
    let mut connections = JoinSet::new();
    while let Ok((socket, _)) = listener.accept().await {
        connections.spawn(serve(socket, tls.clone(), host.clone(), port));
        while connections.try_join_next().is_some() {}
    }
}

async fn serve(mut socket: TcpStream, tls: TlsAcceptor, host: String, port: u16) {
    let mut request = [0_u8; 8];
    if socket.read_exact(&mut request).await.is_err() || request != [0, 0, 0, 8, 4, 210, 22, 47] {
        return;
    }
    if socket.write_all(b"S").await.is_err() {
        return;
    }
    let Ok(mut client) = tls.accept(socket).await else {
        return;
    };
    let mut server = TcpStream::connect((host.as_str(), port)).await.unwrap();
    drop(copy_bidirectional(&mut client, &mut server).await);
}
