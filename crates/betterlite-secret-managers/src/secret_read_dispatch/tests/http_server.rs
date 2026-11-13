use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{Builder, JoinHandle};
use std::time::Duration;

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse as _, Response};
use tokio::net::TcpListener as TokioListener;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::sync::oneshot;
use tokio::time::sleep;

#[derive(Clone)]
pub(super) struct Recorded {
    pub(super) method: Method,
    pub(super) path: String,
    pub(super) headers: HeaderMap,
    pub(super) body: String,
}

#[derive(Clone, Default)]
pub(super) struct Recorder(Arc<Mutex<Vec<Recorded>>>);

impl Recorder {
    pub(super) fn all(&self) -> Vec<Recorded> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: Recorded) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }
}

#[derive(Clone)]
enum Reply {
    Fixed { status: StatusCode, body: String },
    Redirect { to: String, body: String },
    Delayed { delay: Duration, body: String },
}

#[derive(Clone)]
struct AnswerState {
    recorder: Recorder,
    routes: Arc<BTreeMap<String, Reply>>,
    fallback: Reply,
}

pub(super) struct Server {
    address: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub(super) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _sent = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

pub(super) fn serve_fixed(status: u16, body: &str) -> (Server, Recorder) {
    let recorder = Recorder::default();
    let fallback = Reply::Fixed {
        status: StatusCode::from_u16(status).expect("a valid status"),
        body: body.to_owned(),
    };
    (start(recorder.clone(), BTreeMap::new(), fallback), recorder)
}

pub(super) fn serve_slow(delay: Duration, body: &str) -> (Server, Recorder) {
    let recorder = Recorder::default();
    let fallback = Reply::Delayed {
        delay,
        body: body.to_owned(),
    };
    (start(recorder.clone(), BTreeMap::new(), fallback), recorder)
}

pub(super) fn serve_redirect(target: &str, redirect_body: &str, body: &str) -> (Server, Recorder) {
    let recorder = Recorder::default();
    let routes = BTreeMap::from([(
        target.to_owned(),
        Reply::Fixed {
            status: StatusCode::OK,
            body: body.to_owned(),
        },
    )]);
    let fallback = Reply::Redirect {
        to: target.to_owned(),
        body: redirect_body.to_owned(),
    };
    (start(recorder.clone(), routes, fallback), recorder)
}

fn start(recorder: Recorder, routes: BTreeMap<String, Reply>, fallback: Reply) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let address = listener.local_addr().expect("a bound address");
    listener
        .set_nonblocking(true)
        .expect("a nonblocking listener");
    let state = AnswerState {
        recorder,
        routes: Arc::new(routes),
        fallback,
    };
    let app = Router::new().fallback(answer).with_state(state);
    let (shutdown, stopped) = oneshot::channel();
    let thread = Builder::new()
        .name("secret-dispatch-http".to_owned())
        .spawn(move || {
            let runtime = RuntimeBuilder::new_multi_thread()
                .enable_all()
                .build()
                .expect("a test runtime");
            runtime.block_on(async move {
                let serving = TokioListener::from_std(listener).expect("a tokio listener");
                axum::serve(serving, app)
                    .with_graceful_shutdown(async {
                        drop(stopped.await);
                    })
                    .await
                    .expect("a serving app");
            });
        })
        .expect("a server thread");
    Server {
        address,
        shutdown: Some(shutdown),
        thread: Some(thread),
    }
}

async fn answer(State(state): State<AnswerState>, request: Request) -> Response {
    let (parts, request_body) = request.into_parts();
    let bytes = to_bytes(request_body, 0x0001_0000)
        .await
        .unwrap_or_default();
    state.recorder.record(Recorded {
        method: parts.method,
        path: parts.uri.path().to_owned(),
        headers: parts.headers,
        body: String::from_utf8(bytes.to_vec()).unwrap_or_default(),
    });
    let reply = state
        .routes
        .get(parts.uri.path())
        .unwrap_or(&state.fallback)
        .clone();
    match reply {
        Reply::Fixed { status, body } => (status, body).into_response(),
        Reply::Delayed { delay, body } => {
            sleep(delay).await;
            (StatusCode::OK, body).into_response()
        }
        Reply::Redirect { to, body } => {
            let mut response = (StatusCode::FOUND, body).into_response();
            response.headers_mut().insert(
                header::LOCATION,
                HeaderValue::from_str(&to).expect("a valid URL"),
            );
            response
        }
    }
}
