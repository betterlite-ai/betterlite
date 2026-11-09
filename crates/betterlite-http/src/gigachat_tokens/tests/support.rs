use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::gigachat_auth::Settings;
use reqwest::redirect::Policy;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;
use uuid::Uuid;

use crate::gigachat_tokens::cache::TokenCache;
use crate::gigachat_tokens::runtime::GigachatRuntime;

pub(super) fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .expect("an environment-only section loads from an empty document")
        .sections
        .gigachat_auth
}

pub(super) fn client() -> reqwest::Client {
    reqwest::ClientBuilder::new()
        .no_proxy()
        .redirect(Policy::none())
        .build()
        .expect("the test HTTP client builds")
}

pub(super) fn request_uid() -> Uuid {
    Uuid::parse_str("3b241101-e2bb-4255-8caf-4136c566a962").expect("the fixed UUID parses")
}

pub(super) fn text(secret: &SecretString) -> String {
    secret.expose_secret().to_owned()
}

pub(super) struct ScriptedRuntime {
    times: Mutex<VecDeque<i64>>,
    uid: Uuid,
}

impl ScriptedRuntime {
    pub(super) fn new(times: &[i64]) -> Self {
        Self {
            times: Mutex::new(times.iter().copied().collect()),
            uid: request_uid(),
        }
    }
}

impl GigachatRuntime for ScriptedRuntime {
    fn now_millis(&self) -> i64 {
        let mut times = self.times.lock().unwrap_or_else(PoisonError::into_inner);
        if times.len() > 1 {
            return times.pop_front().unwrap_or_default();
        }
        times.front().copied().unwrap_or_default()
    }

    fn request_uid(&self) -> Uuid {
        self.uid
    }
}

pub(super) async fn insert_entries(
    cache: &TokenCache,
    count: i64,
    first_expiry: i64,
    now_millis: i64,
) {
    for index in 0..count {
        cache
            .insert(
                format!("key-{index}"),
                SecretString::from(format!("token-{index}")),
                first_expiry.saturating_add(index),
                now_millis,
            )
            .await;
    }
}

#[derive(Clone)]
pub(super) struct Seen {
    pub(super) method: String,
    pub(super) path: String,
    headers: BTreeMap<String, String>,
    pub(super) body: String,
}

impl Seen {
    pub(super) fn header(&self, name: &str) -> &str {
        self.headers.get(name).map_or("", String::as_str)
    }
}

pub(super) struct Answer {
    status: u16,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}

impl Answer {
    pub(super) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            headers: Vec::new(),
        }
    }

    pub(super) fn bytes(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            body: body.to_vec(),
            headers: Vec::new(),
        }
    }

    pub(super) fn redirect(status: u16, location: &str) -> Self {
        Self {
            status,
            body: Vec::new(),
            headers: vec![("location".to_owned(), location.to_owned())],
        }
    }
}

pub(super) struct Server {
    pub(super) url: String,
    requests: Arc<Mutex<Vec<Seen>>>,
    answers: Arc<Mutex<VecDeque<Answer>>>,
    _tasks: JoinSet<()>,
}

impl Server {
    pub(super) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the test server binds a loopback port");
        let url = format!(
            "http://{}",
            listener.local_addr().expect("the bound address is known")
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(VecDeque::new()));
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(listener, Arc::clone(&requests), Arc::clone(&answers)));
        Self {
            url,
            requests,
            answers,
            _tasks: tasks,
        }
    }

    pub(super) fn answer(&self, status: u16, body: &str) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Answer::json(status, body));
    }

    pub(super) fn answer_bytes(&self, status: u16, body: &[u8]) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Answer::bytes(status, body));
    }

    pub(super) fn answer_redirect(&self, status: u16, location: &str) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Answer::redirect(status, location));
    }

    pub(super) fn requests(&self) -> Vec<Seen> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn serve(
    listener: TcpListener,
    requests: Arc<Mutex<Vec<Seen>>>,
    answers: Arc<Mutex<VecDeque<Answer>>>,
) {
    while let Ok((mut stream, _peer)) = listener.accept().await {
        let Some(seen) = read_request(&mut stream).await else {
            continue;
        };
        requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
        let answer = answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Answer::json(500, ""));
        let mut reply = format!("HTTP/1.1 {} Answer\r\n", answer.status).into_bytes();
        for (name, value) in &answer.headers {
            reply.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        reply.extend_from_slice(
            format!(
                "content-length: {}\r\nconnection: close\r\n\r\n",
                answer.body.len()
            )
            .as_bytes(),
        );
        reply.extend_from_slice(&answer.body);
        drop(stream.write_all(&reply).await);
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        received.extend_from_slice(chunk.get(..read)?);
        let Some(head_end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
            if read == 0 {
                return None;
            }
            continue;
        };
        let head = str::from_utf8(received.get(..head_end)?).ok()?;
        let length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        let body_start = head_end.checked_add(4)?;
        let body_length = received.len().saturating_sub(body_start);
        if body_length >= length || read == 0 {
            let body = String::from_utf8(received.get(body_start..)?.to_vec()).ok()?;
            return parse_seen(head, body);
        }
    }
}

fn parse_seen(head: &str, body: String) -> Option<Seen> {
    let mut lines = head.lines();
    let mut request = lines.next()?.split_whitespace();
    let method = request.next()?.to_owned();
    let path = request.next()?.to_owned();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    Some(Seen {
        method,
        path,
        headers,
        body,
    })
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(super) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let guard = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(logs.clone())
            .with_ansi(false)
            .set_default();
        (logs, guard)
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured log output is UTF-8")
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
