use std::collections::VecDeque;
use std::io;
use std::net::TcpListener;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{Builder as ThreadBuilder, JoinHandle};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use secrecy::ExposeSecret as _;
use tokio::net::TcpListener as TokioListener;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::sync::oneshot;
use tokio::time::sleep;

use super::super::HiddenlayerAuth;
use super::super::token::{JwtError, JwtTransport, TokenRequest};

pub(crate) fn has_token(auth: &HiddenlayerAuth) -> bool {
    auth.jwt.lock().unwrap().is_some()
}

#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) authorization: Option<String>,
    pub(crate) content_type: Option<String>,
    pub(crate) body: String,
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
    pub(crate) delay: Option<Duration>,
}

pub(crate) fn reply(status: u16, body: &str) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body: body.to_owned(),
        delay: None,
    }
}

pub(crate) fn reply_with(status: u16, headers: &[(&str, &str)], body: &str) -> Reply {
    Reply {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: body.to_owned(),
        delay: None,
    }
}

pub(crate) fn delayed(status: u16, body: &str, delay: Duration) -> Reply {
    Reply {
        delay: Some(delay),
        ..reply(status, body)
    }
}

pub(crate) struct Server {
    pub(crate) base: String,
    pub(crate) port: u16,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl Server {
    pub(crate) fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _sent = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

pub(crate) fn start(replies: Vec<Reply>) -> Server {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let answers = Arc::new(Mutex::new(VecDeque::from(replies)));
    let recorder = Arc::clone(&seen);
    let answer = move |method: Method, uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let capture = Arc::clone(&recorder);
        let queue = Arc::clone(&answers);
        async move {
            record(&capture, &method, &uri, &headers, &bytes);
            let next = queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| reply(500, "{}"));
            if let Some(delay) = next.delay {
                sleep(delay).await;
            }
            (
                StatusCode::from_u16(next.status).unwrap(),
                response_headers(&next.headers),
                next.body,
            )
        }
    };
    let (stop, thread) = serve(listener, Router::new().fallback(answer));
    Server {
        base: format!("http://{address}"),
        port: address.port(),
        seen,
        stop: Some(stop),
        thread: Some(thread),
    }
}

fn response_headers(headers: &[(String, String)]) -> HeaderMap {
    let mut response_headers = HeaderMap::new();
    for (header_name, header_value) in headers {
        let name = HeaderName::from_bytes(header_name.as_bytes()).unwrap();
        response_headers.insert(name, HeaderValue::from_str(header_value).unwrap());
    }
    response_headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/json"),
    );
    response_headers
}

fn serve(
    listener: TcpListener,
    router: Router,
) -> (oneshot::Sender<()>, JoinHandle<io::Result<()>>) {
    let (stop, stopped) = oneshot::channel();
    let thread = ThreadBuilder::new()
        .spawn(move || {
            let runtime = RuntimeBuilder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let tokio_listener = TokioListener::from_std(listener).unwrap();
                axum::serve(tokio_listener, router)
                    .with_graceful_shutdown(async move {
                        let _stopped = stopped.await;
                    })
                    .await
            })
        })
        .unwrap();
    (stop, thread)
}

fn record(
    recorder: &Mutex<Vec<Seen>>,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    bytes: &Bytes,
) {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    recorder.lock().unwrap().push(Seen {
        method: method.to_string(),
        path: uri.to_string(),
        authorization: header("authorization"),
        content_type: header("content-type"),
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    });
}

#[derive(Clone, Debug)]
pub(crate) struct SeenTokenRequest {
    pub(crate) url: String,
    pub(crate) api_id: String,
    pub(crate) api_key: String,
    pub(crate) timeout: Duration,
}

#[derive(Default)]
pub(crate) struct TokenScript {
    pub(crate) answers: Mutex<VecDeque<Result<String, JwtError>>>,
    pub(crate) calls: Mutex<Vec<SeenTokenRequest>>,
}

impl TokenScript {
    pub(crate) fn requests(&self) -> Vec<SeenTokenRequest> {
        self.calls.lock().unwrap().clone()
    }
}

impl JwtTransport for TokenScript {
    fn fetch_token(&self, request: &TokenRequest) -> Result<String, JwtError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenTokenRequest {
                url: request.url.clone(),
                api_id: request.api_id.clone(),
                api_key: request.api_key.expose_secret().to_owned(),
                timeout: request.timeout,
            });
        let next = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        next.unwrap_or_else(|| {
            Err(JwtError::Body {
                source: io::Error::other("the token script has no answer left"),
            })
        })
    }
}

pub(crate) fn tokens(values: &[&str]) -> Arc<TokenScript> {
    let answers = values
        .iter()
        .map(|value| Ok((*value).to_owned()))
        .collect::<VecDeque<_>>();
    Arc::new(TokenScript {
        answers: Mutex::new(answers),
        calls: Mutex::new(Vec::new()),
    })
}

pub(crate) fn failing(error: JwtError) -> Arc<TokenScript> {
    let script = TokenScript::default();
    script.answers.lock().unwrap().push_back(Err(error));
    Arc::new(script)
}
