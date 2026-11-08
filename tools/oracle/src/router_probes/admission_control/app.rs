use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{OriginalUri, State};
use axum::response::Response;
use futures_util::stream;
use tokio::sync::Notify;

use super::{Behavior, Input, RequestSpec};

pub(super) struct Gate {
    pub(super) behavior: Behavior,
    pub(super) started: Arc<Notify>,
    pub(super) release: Arc<Notify>,
}

pub(super) struct App {
    pub(super) gates: BTreeMap<String, Gate>,
}

pub(super) async fn dispatch(
    State(app): State<Arc<App>>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let Some(gate) = app.gates.get(uri.path()) else {
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
            gate.started.notify_one();
            let notifier = Arc::clone(&gate.release);
            let body = Body::from_stream(stream::unfold(0_u8, move |step| {
                stream_chunk(step, Arc::clone(&notifier))
            }));
            Response::new(body)
        }
    }
}

async fn stream_chunk(step: u8, release: Arc<Notify>) -> Option<(Result<Bytes, axum::Error>, u8)> {
    match step {
        0 => Some((Ok(Bytes::from_static(b"first")), 1)),
        1 => {
            release.notified().await;
            Some((Ok(Bytes::from_static(b"last")), 2))
        }
        _ => None,
    }
}

pub(super) fn gate_app(input: &Input) -> App {
    let gates = input
        .requests
        .iter()
        .map(|spec| (spec.path.clone(), gate(spec)))
        .collect();
    App { gates }
}

fn gate(spec: &RequestSpec) -> Gate {
    Gate {
        behavior: spec.behavior,
        started: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
    }
}
