use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::TcpListener;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{Builder as ThreadBuilder, JoinHandle};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use tokio::net::TcpListener as TokioListener;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::sync::oneshot;
use tokio::time::sleep;

// One call the guardrail made, with every request header lowered, as the server saw it
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(&'static str, &'static str)>,
    pub(crate) body: String,
    pub(crate) after: Option<Duration>,
}

pub(crate) fn reply(body: &str) -> Reply {
    Reply {
        status: 200,
        headers: Vec::new(),
        body: body.to_owned(),
        after: None,
    }
}

pub(crate) fn reply_with(
    status: u16,
    headers: &[(&'static str, &'static str)],
    body: &str,
) -> Reply {
    Reply {
        status,
        headers: headers.to_vec(),
        body: body.to_owned(),
        after: None,
    }
}

// answers only after the delay, so a caller with a shorter timeout has already stopped waiting
pub(crate) fn stall(delay: Duration) -> Reply {
    Reply {
        after: Some(delay),
        ..reply("{}")
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
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    let listener = local_listener();
    let address = listener.local_addr().expect("an address");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let answers = Arc::new(Mutex::new(VecDeque::from(replies)));
    let recorder = Arc::clone(&seen);
    let handler = move |method: Method, uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let capture = Arc::clone(&recorder);
        let queue = Arc::clone(&answers);
        async move {
            record(&capture, &method, &uri, &headers, &bytes);
            let next = next_reply(&queue);
            if let Some(delay) = next.after {
                sleep(delay).await;
            }
            (
                StatusCode::from_u16(next.status).expect("a status"),
                response_headers(&next.headers),
                next.body,
            )
        }
    };
    let (stop, stopped) = oneshot::channel();
    let router = Router::new().fallback(handler);
    let thread = ThreadBuilder::new()
        .spawn(move || serve(listener, router, stopped))
        .expect("a server thread");
    Server {
        base: format!("http://{address}"),
        port: address.port(),
        seen,
        stop: Some(stop),
        thread: Some(thread),
    }
}

fn local_listener() -> TcpListener {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a listener");
    listener
        .set_nonblocking(true)
        .expect("a nonblocking listener");
    listener
}

fn next_reply(queue: &Mutex<VecDeque<Reply>>) -> Reply {
    queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
        .unwrap_or_else(|| reply("{}"))
}

fn serve(listener: TcpListener, router: Router, stopped: oneshot::Receiver<()>) -> io::Result<()> {
    let runtime = RuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async move {
        let async_listener = TokioListener::from_std(listener).expect("a tokio listener");
        axum::serve(async_listener, router)
            .with_graceful_shutdown(async move {
                let _stopped = stopped.await;
            })
            .await
    })
}

fn response_headers(headers: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut response = HeaderMap::new();
    for (raw_name, raw_value) in headers {
        let name = HeaderName::from_bytes(raw_name.as_bytes()).expect("a header name");
        response.insert(
            name,
            HeaderValue::from_str(raw_value).expect("a header value"),
        );
    }
    response.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/json"),
    );
    response
}

fn record(
    recorder: &Mutex<Vec<Seen>>,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    bytes: &Bytes,
) {
    let mut captured = BTreeMap::new();
    for (name, value) in headers {
        if let Ok(text) = value.to_str() {
            captured.insert(name.as_str().to_owned(), text.to_owned());
        }
    }
    recorder
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Seen {
            method: method.to_string(),
            path: uri.to_string(),
            headers: captured,
            body: String::from_utf8(bytes.to_vec()).expect("a UTF-8 body"),
        });
}
