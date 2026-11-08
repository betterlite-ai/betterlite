use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::Response;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::request_root_paths;
use futures_util::stream;
use tokio::sync::Notify;
use tokio::task::{AbortHandle, JoinSet};
use tokio::time::{sleep, timeout};
use tower::ServiceExt as _;

use crate::admission_control::{AdmissionControl, AdmissionStack, InFlightRequests};
use crate::middleware::request_root_paths::RootPaths;

pub(super) const WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Behavior {
    Immediate,
    Hold,
    Stream,
}

pub(super) struct Gate {
    behavior: Behavior,
    started: Arc<Notify>,
    pub(super) release: Arc<Notify>,
}

pub(super) type Gates = BTreeMap<String, Gate>;

pub(super) struct Harness {
    pub(super) router: Router,
    pub(super) stack: AdmissionStack,
    pub(super) gates: Arc<Gates>,
}

type Answer = (StatusCode, HeaderMap, Bytes);

#[derive(Default)]
pub(super) struct Requests {
    tasks: JoinSet<(String, Answer)>,
    aborts: BTreeMap<String, AbortHandle>,
    finished: BTreeMap<String, Answer>,
}

pub(super) async fn dispatch(
    State(gates): State<Arc<Gates>>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let Some(gate) = gates.get(uri.path()) else {
        return Response::new(Body::from("ok"));
    };
    match gate.behavior {
        Behavior::Immediate => {
            gate.started.notify_one();
            Response::new(Body::from("ok"))
        }
        Behavior::Hold => {
            gate.started.notify_one();
            gate.release.notified().await;
            Response::new(Body::from("ok"))
        }
        Behavior::Stream => {
            let notifier = Arc::clone(&gate.release);
            let started = Arc::clone(&gate.started);
            let body = Body::from_stream(stream::unfold(0_u8, move |step| {
                stream_chunk(step, Arc::clone(&notifier), Arc::clone(&started))
            }));
            Response::new(body)
        }
    }
}

async fn stream_chunk(
    step: u8,
    release: Arc<Notify>,
    started: Arc<Notify>,
) -> Option<(Result<Bytes, IoError>, u8)> {
    match step {
        0 => Some((Ok(Bytes::from_static(b"first")), 1)),
        1 => {
            started.notify_one();
            release.notified().await;
            Some((Ok(Bytes::from_static(b"last")), 2))
        }
        _ => None,
    }
}

pub(super) fn gates(specs: &[(&str, Behavior)]) -> Gates {
    specs
        .iter()
        .map(|(path, behavior)| {
            (
                (*path).to_owned(),
                Gate {
                    behavior: *behavior,
                    started: Arc::new(Notify::new()),
                    release: Arc::new(Notify::new()),
                },
            )
        })
        .collect()
}

pub(super) fn harness(source: &str, root_path: &str, configured: Gates) -> Harness {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap();
    let stack = AdmissionStack {
        admission: Arc::new(AdmissionControl::from_settings(
            &config.sections.admission_control,
        )),
        in_flight: Arc::new(InFlightRequests::default()),
        scalar_root: config.sections.request_root_paths.server_root_path.clone(),
    };
    let (router, gates) = application(
        &config.sections.request_root_paths,
        root_path,
        configured,
        |inner| stack.wrap(inner),
    );
    Harness {
        router,
        stack,
        gates,
    }
}

pub(super) fn application<F>(
    root_settings: &request_root_paths::Settings,
    root_path: &str,
    configured: Gates,
    wrap: F,
) -> (Router, Arc<Gates>)
where
    F: FnOnce(Router) -> Router,
{
    let gates = Arc::new(configured);
    let inner = Router::new()
        .fallback(dispatch)
        .with_state(Arc::clone(&gates));
    let stacked = wrap(inner);
    let mut roots = root_settings.clone();
    roots.server_root_paths = vec![root_path.to_owned()];
    let routed = match RootPaths::from_settings(&roots) {
        Some(paths) => paths.wrap(stacked),
        None => stacked,
    };
    (routed, gates)
}

async fn call(router: Router, path: String, upgrade: bool) -> Answer {
    let mut builder = Request::builder().uri(path);
    if upgrade {
        builder = builder
            .header(header::UPGRADE, "websocket")
            .header(header::CONNECTION, "keep-alive, Upgrade")
            .header(header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
            .header(header::SEC_WEBSOCKET_VERSION, "13");
    }
    let request = builder.body(Body::empty()).unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, headers, body)
}

async fn call_later(router: Router, path: String, upgrade: bool) -> (String, Answer) {
    let answer = call(router, path.clone(), upgrade).await;
    (path, answer)
}

impl Requests {
    pub(super) fn start(&mut self, router: &Router, path: &str) {
        let abort = self
            .tasks
            .spawn(call_later(router.clone(), path.to_owned(), false));
        self.aborts.insert(path.to_owned(), abort);
    }

    pub(super) fn start_upgrade(&mut self, router: &Router, path: &str) {
        let abort = self
            .tasks
            .spawn(call_later(router.clone(), path.to_owned(), true));
        self.aborts.insert(path.to_owned(), abort);
    }

    pub(super) async fn cancel(&mut self, path: &str) {
        let Some(abort) = self.aborts.remove(path) else {
            return;
        };
        abort.abort();
        while let Some(joined) = self.tasks.join_next().await {
            let Ok((finished_path, answer)) = joined else {
                return;
            };
            self.finished.insert(finished_path, answer);
        }
    }

    pub(super) async fn complete(&mut self, path: &str) -> Answer {
        if let Some(answer) = self.finished.remove(path) {
            return answer;
        }
        while let Some(joined) = self.tasks.join_next().await {
            let Ok((finished_path, answer)) = joined else {
                panic!("the request for {path} failed to join")
            };
            if finished_path == path {
                return answer;
            }
            self.finished.insert(finished_path, answer);
        }
        panic!("the request for {path} never completed")
    }
}

pub(super) async fn await_started(gates: &Gates, path: &str) {
    let gate = gates.get(path).unwrap();
    timeout(WAIT, gate.started.notified()).await.unwrap();
}

pub(super) async fn await_queued(stack: &AdmissionStack, queued: u64) {
    timeout(WAIT, async {
        while stack.admission.stats().queued != queued {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
