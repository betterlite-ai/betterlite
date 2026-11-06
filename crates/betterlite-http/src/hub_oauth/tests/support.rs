use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{OfflineSetting, Settings, TransferSetting};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::hub_oauth::clock::HubClock;
use crate::hub_token::{
    HubToken, HubTokenError, HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader,
};
use crate::proxy::ProxiedClient;

pub(super) const HANG: u16 = 0;
pub(super) const NOW: i64 = 1_700_000_000;

pub(super) struct FakeClock {
    monotonic: AtomicU64,
    unix: AtomicI64,
}

impl FakeClock {
    pub(super) fn new(unix: i64) -> Arc<Self> {
        Arc::new(Self {
            monotonic: AtomicU64::new(0),
            unix: AtomicI64::new(unix),
        })
    }

    pub(super) fn advance(&self, seconds: u64) {
        self.monotonic.fetch_add(seconds, Ordering::SeqCst);
        self.unix
            .fetch_add(i64::try_from(seconds).unwrap(), Ordering::SeqCst);
    }
}

impl HubClock for FakeClock {
    fn monotonic_seconds(&self) -> u64 {
        self.monotonic.load(Ordering::SeqCst)
    }

    fn unix_seconds(&self) -> i64 {
        self.unix.load(Ordering::SeqCst)
    }
}

pub(super) struct Server {
    pub(super) endpoint: String,
    requests: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<VecDeque<(u16, String)>>>,
    _tasks: JoinSet<()>,
}

impl Server {
    pub(super) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(VecDeque::new()));
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(listener, Arc::clone(&requests), Arc::clone(&answers)));
        Self {
            endpoint,
            requests,
            answers,
            _tasks: tasks,
        }
    }

    pub(super) fn answer(&self, status: u16, body: &str) {
        self.answers
            .lock()
            .unwrap()
            .push_back((status, body.to_owned()));
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

async fn serve(
    listener: TcpListener,
    requests: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<VecDeque<(u16, String)>>>,
) {
    while let Ok((mut stream, _peer)) = listener.accept().await {
        let request = read_request(&mut stream).await;
        requests.lock().unwrap().push(request);
        let (status, body) = answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or((500, String::new()));
        if status == HANG {
            sleep(Duration::from_secs(60)).await;
        }
        let reply = format!(
            "HTTP/1.1 {status} Answer\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(reply.as_bytes()).await.unwrap();
    }
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        received.extend_from_slice(chunk.get(..read).unwrap());
        let head_end = received.windows(4).position(|window| window == b"\r\n\r\n");
        if let Some(position) = head_end {
            let head = str::from_utf8(&received[..position]).unwrap();
            let length: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .map_or(0, |value| value.trim().parse().unwrap());
            let body_length = received.len().saturating_sub(position).saturating_sub(4);
            if body_length >= length || read == 0 {
                return String::from_utf8(received).unwrap();
            }
        }
    }
}

pub(super) fn body_of(request: &str) -> &str {
    request.split_once("\r\n\r\n").unwrap().1
}

pub(super) struct Env {
    pub(super) directory: TempDir,
    pub(super) server: Server,
    pub(super) clock: Arc<FakeClock>,
}

impl Env {
    pub(super) async fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
            server: Server::start().await,
            clock: FakeClock::new(NOW),
        }
    }

    pub(super) fn hub(&self) -> Settings {
        let hub_directory = self.directory.path().join("hub");
        Settings {
            endpoint: self.server.endpoint.clone(),
            offline: OfflineSetting::Online,
            etag_timeout: 10,
            download_timeout: 10,
            disable_implicit_token: false,
            env_token: None,
            hf_home: self.directory.path().to_path_buf(),
            hub_cache: self.directory.path().to_path_buf(),
            token_path: hub_directory.join("token"),
            stored_tokens_path: hub_directory.join("stored_tokens"),
            transfer: TransferSetting::Standard,
        }
    }

    pub(super) fn lookup(&self, hub: Settings, oidc: OidcSettings) -> HubTokenResolver {
        self.lookup_with_reader(hub, oidc, Arc::new(TokioHubTokenFileReader))
    }

    pub(super) fn lookup_with_reader(
        &self,
        hub: Settings,
        oidc: OidcSettings,
        reader: Arc<dyn HubTokenFileReader>,
    ) -> HubTokenResolver {
        let policy =
            Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
        let clock: Arc<dyn HubClock> = Arc::<FakeClock>::clone(&self.clock);
        HubTokenResolver::new(
            hub,
            oidc,
            Arc::new(ProxiedClient::outbound(&policy).unwrap()),
            reader,
            clock,
        )
    }
}

pub(super) fn oidc_settings(resource: Option<&str>) -> OidcSettings {
    OidcSettings {
        resource: resource.map(str::to_owned),
        id_token: None,
        github_actions: false,
        request_url: None,
        request_token: None,
        device_code_client_id: "26be6b09-91c5-47da-9861-d2d2bb7a7e36".to_owned(),
    }
}

pub(super) async fn implicit(resolver: &HubTokenResolver) -> Result<Option<String>, HubTokenError> {
    let token = resolver.resolve(&HubToken::Implicit).await?;
    Ok(token.map(|secret: SecretString| secret.expose_secret().to_owned()))
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(super) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let guard = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .set_default();
        (logs, guard)
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut logged = self
            .0
            .lock()
            .map_err(|_poisoned| io::Error::other("log buffer poisoned"))?;
        logged.extend_from_slice(buf);
        drop(logged);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn header<'request>(request: &'request str, name: &str) -> Option<&'request str> {
    request
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
}
