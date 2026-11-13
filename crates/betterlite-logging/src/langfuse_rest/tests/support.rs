use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::langfuse_rest::cache::Clock;
use crate::langfuse_rest::client::{
    LangfuseApiClient, LangfuseClientConfig, build_langfuse_client,
};
use crate::langfuse_rest::errors::{LangfusePromptError, LangfuseRestError};
use crate::langfuse_rest::records::Prompt;

pub(crate) const PUBLIC_KEY: &str = "pk-lf-test";
pub(crate) const SECRET_KEY: &str = "sk-lf-test";
pub(crate) const PROJECTS: &str = r#"{"data":[{"id":"project-one","name":"One","organization":{"id":"org-1","name":"Org"},"metadata":{"tier":"pro"}},{"id":"project-two","name":"Two","organization":{"id":"org-1","name":"Org"},"metadata":{}}]}"#;
pub(crate) const NO_PROJECTS: &str = r#"{"data":[]}"#;

pub(crate) enum Answer {
    Respond {
        status: u16,
        body: String,
    },
    RespondWith {
        status: u16,
        body: String,
        headers: Vec<(&'static str, String)>,
    },
    Close,
    Hold {
        release: oneshot::Receiver<()>,
        status: u16,
        body: String,
    },
    Raw {
        status: u16,
        body: Vec<u8>,
    },
    Truncated {
        body: Vec<u8>,
        headers: Vec<(&'static str, String)>,
    },
    Stall(Duration),
}

pub(crate) struct FakeLangfuse {
    pub(crate) base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<()>,
}

impl FakeLangfuse {
    pub(crate) async fn start(script: Vec<Answer>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(VecDeque::from(script)));
        let mut tasks = JoinSet::new();
        let seen = Arc::clone(&requests);
        tasks.spawn(async move {
            let mut connections = JoinSet::new();
            while let Ok((stream, _peer)) = listener.accept().await {
                connections.spawn(serve(stream, Arc::clone(&seen), Arc::clone(&answers)));
            }
        });
        Self {
            base_url: format!("http://{address}"),
            requests,
            _tasks: tasks,
        }
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(crate) async fn wait_for_requests(&self, count: usize) {
        let patience = Duration::from_secs(5);
        let started = Instant::now();
        while self.request_count() < count {
            assert!(
                started.elapsed() < patience,
                "the server never received {count} requests"
            );
            sleep(Duration::from_millis(5)).await;
        }
    }
}

async fn serve(
    mut stream: TcpStream,
    requests: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<VecDeque<Answer>>>,
) {
    let head = read_head(&mut stream).await;
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(head);
    let next = answers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front();
    let Some(answer) = next else {
        return;
    };
    answer_request(&mut stream, answer).await;
}

async fn answer_request(stream: &mut TcpStream, answer: Answer) {
    match answer {
        Answer::Respond { status, body } => {
            respond(stream, status, body.as_bytes(), &[], 0).await;
        }
        Answer::RespondWith {
            status,
            body,
            headers,
        } => respond(stream, status, body.as_bytes(), &headers, 0).await,
        Answer::Close => {
            let _closed = stream.shutdown().await;
        }
        Answer::Hold {
            release,
            status,
            body,
        } => {
            let _released = release.await;
            respond(stream, status, body.as_bytes(), &[], 0).await;
        }
        Answer::Raw { status, body } => respond(stream, status, &body, &[], 0).await,
        // 16 bytes more than are sent, so the connection ends in the middle of the body
        Answer::Truncated { body, headers } => respond(stream, 200, &body, &headers, 16).await,
        Answer::Stall(delay) => {
            sleep(delay).await;
            respond(stream, 200, b"", &[], 0).await;
        }
    }
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read_exact(&mut byte).await.is_err() {
            break;
        }
        head.push(byte[0]);
    }
    String::from_utf8(head).expect("the request head is UTF-8")
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    body: &[u8],
    headers: &[(&'static str, String)],
    declared_extra: usize,
) {
    let reason = http::StatusCode::from_u16(status)
        .expect("a status")
        .canonical_reason()
        .unwrap_or_default();
    let declared = body.len().saturating_add(declared_extra);
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {declared}\r\nconnection: close\r\n"
    );
    for (name, value) in headers {
        let _formatted = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    let _head_written = stream.write_all(head.as_bytes()).await;
    let _body_written = stream.write_all(body).await;
    let _closed = stream.shutdown().await;
}

#[derive(Clone)]
pub(crate) struct FakeClock(pub(crate) Arc<Mutex<Instant>>);

impl FakeClock {
    pub(crate) fn advance(&self, duration: Duration) {
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let advanced = now
            .checked_add(duration)
            .expect("the fake clock does not overflow");
        *now = advanced;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) fn proxied() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the proxy policy resolves");
    ProxiedClient::outbound(&Arc::new(policy)).expect("the client builds")
}

pub(crate) struct ClientSetup<'settings> {
    pub base_url: String,
    pub public_key: Option<&'settings str>,
    pub secret_key: Option<&'settings str>,
    pub ttl: Duration,
    pub timeout: Duration,
    pub http: Option<ProxiedClient>,
    pub clock: &'settings FakeClock,
    pub tasks: &'settings TaskTracker,
}

pub(crate) fn client_for(
    server: &FakeLangfuse,
    ttl: Duration,
    clock: &FakeClock,
    tasks: &TaskTracker,
) -> LangfuseApiClient {
    client_from(ClientSetup {
        base_url: server.base_url.clone(),
        public_key: Some(PUBLIC_KEY),
        secret_key: Some(SECRET_KEY),
        ttl,
        timeout: Duration::from_secs(5),
        http: Some(proxied()),
        clock,
        tasks,
    })
}

pub(crate) fn client_from(setup: ClientSetup<'_>) -> LangfuseApiClient {
    build_langfuse_client(
        LangfuseClientConfig {
            public_key: setup.public_key.map(str::to_owned),
            secret_key: setup.secret_key.map(SecretString::from),
            base_url: setup.base_url,
            timeout: setup.timeout,
            prompt_cache_ttl: setup.ttl,
        },
        setup.http,
        Arc::new(setup.clock.clone()),
        setup.tasks.clone(),
    )
    .expect("the client builds")
}

pub(crate) fn text_prompt(name: &str, version: i64) -> String {
    format!(
        r#"{{"type":"text","prompt":"Hello {{{{name}}}}","name":"{name}","version":{version},"config":{{"model":"gpt-4o"}},"labels":["production"],"tags":["greeting"],"commitMessage":"first","resolutionGraph":{{"prompt":"greeting"}}}}"#
    )
}

pub(crate) fn chat_prompt(name: &str, version: i64) -> String {
    format!(
        r#"{{"type":"chat","prompt":[{{"role":"system","content":"You help"}},{{"name":"history"}}],"name":"{name}","version":{version},"config":{{}},"labels":[],"tags":[]}}"#
    )
}

pub(crate) fn api_error(error: LangfuseRestError) -> LangfusePromptError {
    match error {
        LangfuseRestError::Api(api) => api,
        LangfuseRestError::Request(failure) => {
            panic!("expected an api error, got {failure:?}")
        }
        LangfuseRestError::Decode { source } => {
            panic!("expected an api error, got {source:?}")
        }
        LangfuseRestError::Body { status, source } => {
            panic!("expected an api error, got status {status}: {source:?}")
        }
    }
}

pub(crate) fn prompt_version(prompt: &Prompt) -> i64 {
    match prompt {
        Prompt::Chat(chat) => chat.meta.version,
        Prompt::Text(text) => text.meta.version,
    }
}

pub(crate) fn header(head: &str, wanted: &str) -> Option<String> {
    head.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(wanted)
            .then(|| value.trim().to_owned())
    })
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("captured logs are UTF-8")
    }
}

impl io::Write for Captured {
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

impl<'writer> MakeWriter<'writer> for Captured {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
