use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, Write as _};
use std::mem;
use std::net::SocketAddr;
use std::str;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rcgen::{BasicConstraints, CertificateParams, CertifiedKey, IsCa, Issuer, KeyPair};
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{WantsServerCert, WebPkiClientVerifier};
use rustls::{ConfigBuilder, RootCertStore, ServerConfig};
use tempfile::NamedTempFile;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite,
    AsyncWriteExt as _, BufReader, split,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;

pub(crate) type ClusterSlots = Vec<(u16, u16, Vec<(String, u16)>)>;

#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    pub(crate) name: String,
    pub(crate) args: Vec<String>,
}

#[derive(Clone)]
pub(crate) enum Reply {
    Simple(String),
    Bulk(String),
    Int(i64),
    Nil,
    Error(String),
    Array(Vec<Self>),
    EchoArgs,
}

impl Reply {
    fn encode(&self, args: &[Vec<u8>]) -> Vec<u8> {
        match self {
            Self::Simple(text) => format!("+{text}\r\n").into_bytes(),
            Self::Bulk(text) => format!("${}\r\n{text}\r\n", text.len()).into_bytes(),
            Self::Int(value) => format!(":{value}\r\n").into_bytes(),
            Self::Nil => b"$-1\r\n".to_vec(),
            Self::Error(text) => format!("-{text}\r\n").into_bytes(),
            Self::Array(items) => {
                let mut out = format!("*{}\r\n", items.len()).into_bytes();
                for item in items {
                    out.extend(item.encode(args));
                }
                out
            }
            Self::EchoArgs => {
                let keys: Vec<&Vec<u8>> = args.iter().skip(1).collect();
                let mut out = format!("*{}\r\n", keys.len()).into_bytes();
                for key in keys {
                    out.extend(format!("${}\r\n", key.len()).into_bytes());
                    out.extend(key);
                    out.extend(b"\r\n");
                }
                out
            }
        }
    }
}

struct Command {
    name: String,
    args: Vec<Vec<u8>>,
}

#[derive(Default)]
struct Session {
    in_multi: bool,
    queued: Vec<Vec<u8>>,
}

#[derive(Default)]
struct FakeState {
    commands: Mutex<Vec<Recorded>>,
    connections: AtomicUsize,
    tls_connections: AtomicUsize,
    script: Mutex<BTreeMap<String, VecDeque<Reply>>>,
    silent: Mutex<BTreeSet<String>>,
    close_after: Mutex<BTreeSet<String>>,
    mute_all: AtomicBool,
    reject_username_auth: AtomicBool,
    cluster_slots: Mutex<Option<ClusterSlots>>,
    sentinel_master: Mutex<Option<(String, u16)>>,
}

pub(crate) struct FakeServer {
    addr: SocketAddr,
    state: Arc<FakeState>,
    tasks: JoinSet<()>,
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

impl FakeServer {
    pub(crate) async fn start() -> Self {
        start_server(None).await
    }

    pub(crate) async fn start_tls(config: Arc<ServerConfig>) -> Self {
        start_server(Some(config)).await
    }

    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub(crate) fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    pub(crate) fn port(&self) -> u16 {
        self.addr.port()
    }

    pub(crate) fn script(&self, name: &str, reply: Reply) {
        let mut script = self.state.script.lock().expect("the fake script lock");
        script
            .entry(name.to_ascii_uppercase())
            .or_default()
            .push_back(reply);
    }

    pub(crate) fn silence(&self, name: &str) {
        let mut silent = self.state.silent.lock().expect("the silent set");
        silent.insert(name.to_ascii_uppercase());
    }

    pub(crate) fn close_after(&self, name: &str) {
        let mut close = self.state.close_after.lock().expect("the close set");
        close.insert(name.to_ascii_uppercase());
    }

    pub(crate) fn mute(&self) {
        self.state.mute_all.store(true, Ordering::Relaxed);
    }

    pub(crate) fn reject_username_auth(&self) {
        self.state
            .reject_username_auth
            .store(true, Ordering::Relaxed);
    }

    pub(crate) fn set_cluster_slots(&self, slots: ClusterSlots) {
        let mut stored = self.state.cluster_slots.lock().expect("the slot map");
        *stored = Some(slots);
    }

    pub(crate) fn set_sentinel_master(&self, host: String, port: u16) {
        let mut stored = self
            .state
            .sentinel_master
            .lock()
            .expect("the master address");
        *stored = Some((host, port));
    }

    pub(crate) fn clear_sentinel_master(&self) {
        let mut stored = self
            .state
            .sentinel_master
            .lock()
            .expect("the master address");
        *stored = None;
    }

    pub(crate) fn commands(&self) -> Vec<Recorded> {
        self.state.commands.lock().expect("the command log").clone()
    }

    pub(crate) fn commands_named(&self, name: &str) -> Vec<Recorded> {
        let wanted = name.to_ascii_uppercase();
        self.commands()
            .into_iter()
            .filter(|recorded| recorded.name == wanted)
            .collect()
    }

    pub(crate) fn connections(&self) -> usize {
        self.state.connections.load(Ordering::Relaxed)
    }

    pub(crate) fn tls_connections(&self) -> usize {
        self.state.tls_connections.load(Ordering::Relaxed)
    }

    pub(crate) fn admitted_client_certificate(&self) -> bool {
        self.tls_connections() > 0
    }
}

async fn start_server(tls: Option<Arc<ServerConfig>>) -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral listener");
    let addr = listener.local_addr().expect("the listener address");
    let state = Arc::new(FakeState::default());
    let mut tasks = JoinSet::new();
    let shared = Arc::clone(&state);
    tasks.spawn(async move { accept_loop(listener, shared, tls).await });
    FakeServer { addr, state, tasks }
}

async fn accept_loop(listener: TcpListener, state: Arc<FakeState>, tls: Option<Arc<ServerConfig>>) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((socket, _peer)) = listener.accept().await else {
            return;
        };
        state.connections.fetch_add(1, Ordering::Relaxed);
        let shared = Arc::clone(&state);
        let acceptor = tls.clone();
        connections.spawn(async move { serve_connection(socket, shared, acceptor).await });
        while connections.try_join_next().is_some() {}
    }
}

async fn serve_connection(
    socket: TcpStream,
    state: Arc<FakeState>,
    tls: Option<Arc<ServerConfig>>,
) {
    let Some(config) = tls else {
        serve(socket, state).await;
        return;
    };
    let Ok(stream) = TlsAcceptor::from(config).accept(socket).await else {
        return;
    };
    state.tls_connections.fetch_add(1, Ordering::Relaxed);
    serve(stream, state).await;
}

async fn serve<S>(stream: S, state: Arc<FakeState>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, mut write_half) = split(stream);
    let mut reader = BufReader::new(read_half);
    let mut session = Session::default();
    while let Ok(Some(command)) = read_command(&mut reader).await {
        state.record(&command);
        if state.mute_all.load(Ordering::Relaxed) {
            continue;
        }
        for reply in state.reply_for(&command, &mut session) {
            if write_half.write_all(&reply).await.is_err() {
                return;
            }
        }
        if write_half.flush().await.is_err() {
            return;
        }
        if state.close_after_command(&command.name) {
            return;
        }
    }
}

async fn read_command<R>(reader: &mut R) -> io::Result<Option<Command>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let Some(count_text) = line.trim().strip_prefix('*') else {
        return Ok(None);
    };
    let count: usize = count_text
        .parse()
        .map_err(|_error| io::Error::other("the fake needs an array command"))?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let mut header = String::new();
        if reader.read_line(&mut header).await? == 0 {
            return Ok(None);
        }
        let Some(length_text) = header.trim().strip_prefix('$') else {
            return Ok(None);
        };
        let length: usize = length_text
            .parse()
            .map_err(|_error| io::Error::other("the fake needs a bulk string"))?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer).await?;
        buffer.truncate(length);
        args.push(buffer);
    }
    let name = args
        .first()
        .and_then(|arg| str::from_utf8(arg).ok())
        .expect("a command name is UTF-8")
        .to_ascii_uppercase();
    Ok(Some(Command { name, args }))
}

fn encode_slot(slot: (u16, u16, Vec<(String, u16)>)) -> Vec<u8> {
    let (start, end, nodes) = slot;
    let mut out = format!(
        "*{}\r\n:{start}\r\n:{end}\r\n",
        nodes.len().saturating_add(2)
    )
    .into_bytes();
    for (host, port) in nodes {
        out.extend(format!("*2\r\n${}\r\n{host}\r\n:{port}\r\n", host.len()).into_bytes());
    }
    out
}

impl FakeState {
    fn close_after_command(&self, name: &str) -> bool {
        let mut close = self.close_after.lock().expect("the close set");
        close.remove(name)
    }

    fn record(&self, command: &Command) {
        let args = command
            .args
            .iter()
            .map(|arg| String::from_utf8(arg.clone()).expect("a command argument is UTF-8"))
            .collect();
        let mut commands = self.commands.lock().expect("the command log");
        commands.push(Recorded {
            name: command.name.clone(),
            args,
        });
    }

    fn reply_for(&self, command: &Command, session: &mut Session) -> Vec<Vec<u8>> {
        match command.name.as_str() {
            "MULTI" => {
                session.in_multi = true;
                session.queued.clear();
                vec![b"+OK\r\n".to_vec()]
            }
            "EXEC" => {
                session.in_multi = false;
                let queued = mem::take(&mut session.queued);
                let mut out = format!("*{}\r\n", queued.len()).into_bytes();
                for reply in queued {
                    out.extend(reply);
                }
                vec![out]
            }
            _ => {
                let reply = self.reply(command);
                if session.in_multi {
                    session.queued.push(reply);
                    vec![b"+QUEUED\r\n".to_vec()]
                } else {
                    vec![reply]
                }
            }
        }
    }

    fn reply(&self, command: &Command) -> Vec<u8> {
        if self
            .silent
            .lock()
            .expect("the silent set")
            .contains(&command.name)
        {
            return Vec::new();
        }
        if let Some(scripted) = self.scripted_reply(&command.name) {
            return scripted.encode(&command.args);
        }
        match command.name.as_str() {
            "AUTH" => self.auth_reply(command),
            "PING" => b"+PONG\r\n".to_vec(),
            "CLUSTER" => self.cluster_reply(command),
            "SENTINEL" => Self::sentinel_reply(self),
            _ => b"+OK\r\n".to_vec(),
        }
    }

    fn scripted_reply(&self, name: &str) -> Option<Reply> {
        self.script
            .lock()
            .expect("the fake script lock")
            .get_mut(name)?
            .pop_front()
    }

    fn auth_reply(&self, command: &Command) -> Vec<u8> {
        if self.reject_username_auth.load(Ordering::Relaxed) && command.args.len() > 2 {
            return b"-ERR wrong number of arguments for 'auth' command\r\n".to_vec();
        }
        b"+OK\r\n".to_vec()
    }

    fn cluster_reply(&self, command: &Command) -> Vec<u8> {
        let sub = command.args.get(1).and_then(|arg| str::from_utf8(arg).ok());
        if sub != Some("SLOTS") {
            return b"+OK\r\n".to_vec();
        }
        let slots = self
            .cluster_slots
            .lock()
            .expect("the slot map")
            .clone()
            .unwrap_or_default();
        let mut out = format!("*{}\r\n", slots.len()).into_bytes();
        for slot in slots {
            out.extend(encode_slot(slot));
        }
        out
    }

    fn sentinel_reply(&self) -> Vec<u8> {
        let master = self
            .sentinel_master
            .lock()
            .expect("the master address")
            .clone();
        match master {
            Some((host, port)) => {
                let port_text = port.to_string();
                format!(
                    "*2\r\n${}\r\n{host}\r\n${}\r\n{port_text}\r\n",
                    host.len(),
                    port_text.len()
                )
                .into_bytes()
            }
            None => b"*-1\r\n".to_vec(),
        }
    }
}

pub(crate) struct TestCertificate {
    pub(crate) cert: CertificateDer<'static>,
    key_der: Vec<u8>,
    pub(crate) cert_pem: String,
}

pub(crate) fn test_certificate(names: &[&str]) -> TestCertificate {
    let owned: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    let CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(owned).expect("a self-signed certificate");
    TestCertificate {
        cert: cert.der().clone(),
        key_der: signing_key.serialize_der(),
        cert_pem: cert.pem(),
    }
}

pub(crate) struct MutualTls {
    pub(crate) ca: TestCertificate,
    pub(crate) cert_pem: String,
    pub(crate) key_pem: String,
}

pub(crate) fn mutual_tls() -> MutualTls {
    let ca_key = KeyPair::generate().expect("a CA key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).expect("a CA certificate");
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let client_key = KeyPair::generate().expect("a client key");
    let client_params =
        CertificateParams::new(vec!["client".to_owned()]).expect("client parameters");
    let client_cert = client_params
        .signed_by(&client_key, &issuer)
        .expect("a signed client certificate");
    MutualTls {
        ca: TestCertificate {
            cert: ca_cert.der().clone(),
            key_der: ca_key.serialize_der(),
            cert_pem: ca_cert.pem(),
        },
        cert_pem: client_cert.pem(),
        key_pem: client_key.serialize_pem(),
    }
}

pub(crate) fn server_config(
    cert: &TestCertificate,
    client_ca: Option<&TestCertificate>,
) -> Arc<ServerConfig> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let versions = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("the default protocol versions");
    let verifier = match client_ca {
        Some(ca) => {
            let mut roots = RootCertStore::empty();
            roots.add(ca.cert.clone()).expect("a client trust anchor");
            WebPkiClientVerifier::builder(Arc::new(roots))
                .build()
                .expect("a client verifier")
        }
        None => return with_server_cert(versions.with_no_client_auth(), cert),
    };
    with_server_cert(versions.with_client_cert_verifier(verifier), cert)
}

fn with_server_cert(
    builder: ConfigBuilder<ServerConfig, WantsServerCert>,
    cert: &TestCertificate,
) -> Arc<ServerConfig> {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_der.clone()));
    Arc::new(
        builder
            .with_single_cert(vec![cert.cert.clone()], key)
            .expect("a TLS server config"),
    )
}

pub(crate) fn temp_pem(contents: &str) -> NamedTempFile {
    let mut file = tempfile::Builder::new()
        .suffix(".pem")
        .tempfile()
        .expect("a temporary PEM file");
    file.write_all(contents.as_bytes())
        .expect("a written PEM file");
    file
}
