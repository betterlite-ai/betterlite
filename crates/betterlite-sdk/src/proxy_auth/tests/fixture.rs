use std::collections::{BTreeMap, VecDeque};
use std::future::ready;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use secrecy::SecretString;
use thiserror::Error;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use url::Url;

use crate::proxy_auth::clock::Clock;
use crate::proxy_auth::{AccessToken, TokenCredential};

#[derive(Debug, Clone, Default)]
pub(super) struct FixedClock(pub(super) Arc<AtomicI64>);

impl Clock for FixedClock {
    fn unix_seconds(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl FixedClock {
    pub(super) fn at(seconds: i64) -> Self {
        Self(Arc::new(AtomicI64::new(seconds)))
    }

    pub(super) fn set(&self, seconds: i64) {
        self.0.store(seconds, Ordering::SeqCst);
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
pub(super) struct Refused(pub(super) String);

pub(super) fn access(token: &str, expires_on: i64) -> AccessToken {
    AccessToken {
        token: SecretString::from(token),
        expires_on,
    }
}

#[derive(Debug, Default)]
pub(super) struct Scripted {
    pub(super) replies: Mutex<VecDeque<Result<AccessToken, Refused>>>,
    pub(super) scopes: Mutex<Vec<String>>,
    pub(super) calls: AtomicUsize,
}

impl Scripted {
    pub(super) fn answering(replies: Vec<Result<AccessToken, Refused>>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            scopes: Mutex::default(),
            calls: AtomicUsize::default(),
        }
    }
}

impl TokenCredential for Scripted {
    type Error = Refused;

    fn get_token(&self, scope: &str) -> impl Future<Output = Result<AccessToken, Refused>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(scope.to_owned());
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Err(Refused("no reply left".to_owned())));
        ready(reply)
    }
}

pub(super) struct TokenServer {
    pub(super) url: Url,
    pub(super) requests: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<()>,
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        received.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8(received.clone()).unwrap();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let wanted = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|line| line.strip_prefix("content-length: ").map(str::to_owned))
                .map_or(0, |length| length.parse::<usize>().unwrap());
            if body.len() >= wanted || read == 0 {
                return text;
            }
        }
    }
}

pub(super) async fn serve(status: u16, body: &str) -> TokenServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = Url::parse(&format!("http://{address}/oauth2/token")).unwrap();
    let requests: Arc<Mutex<Vec<String>>> = Arc::default();
    let length = body.len();
    let answer = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {length}\r\nconnection: close\r\n\r\n{body}"
    );
    let seen = Arc::clone(&requests);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let text = read_request(&mut stream).await;
            seen.lock().unwrap().push(text);
            stream.write_all(answer.as_bytes()).await.unwrap();
        }
    });
    TokenServer {
        url,
        requests,
        _tasks: tasks,
    }
}

pub(super) fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}
