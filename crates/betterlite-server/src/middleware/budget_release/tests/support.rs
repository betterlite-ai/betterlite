use std::future::Future;
use std::io;
use std::pin::Pin;
use std::str;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_trait::async_trait;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, Response, StatusCode};
use futures_util::stream;
use tokio::sync::Barrier;
use tower::Service;
use tracing_subscriber::fmt::MakeWriter;

use crate::middleware::budget_release::ReservationReleaser;
use crate::middleware::budget_release::slot::ReservationSlot;

const STAMP_HEADER: &str = "x-test-reservation";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Stamp {
    pub(super) id: u32,
    pub(super) callback_bound: bool,
}

pub(super) type Events = Arc<Mutex<Vec<String>>>;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("the inner service failed")]
pub(super) struct InnerFailed;

#[derive(Debug, thiserror::Error)]
#[error("the spend counters are unreachable")]
pub(super) struct ReleaseRefused;

pub(super) struct Recorder {
    pub(super) released: Mutex<Vec<Stamp>>,
    pub(super) events: Events,
    pub(super) refuse: bool,
}

impl Recorder {
    pub(super) fn released(&self) -> Vec<Stamp> {
        self.released.lock().unwrap().clone()
    }
}

#[async_trait]
impl ReservationReleaser for Recorder {
    type Reservation = Stamp;
    type Error = ReleaseRefused;

    async fn release(&self, reservation: &Stamp) -> Result<(), ReleaseRefused> {
        self.released.lock().unwrap().push(reservation.clone());
        self.events
            .lock()
            .unwrap()
            .push(format!("released {}", reservation.id));
        if self.refuse {
            return Err(ReleaseRefused);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Reply {
    Plain,
    ServerError,
    Failed,
    Streaming,
}

#[derive(Clone)]
pub(super) struct Handler {
    pub(super) reply: Reply,
    pub(super) events: Events,
    pub(super) callback_bound: bool,
    pub(super) barrier: Option<Arc<Barrier>>,
}

impl Service<Request<Body>> for Handler {
    type Response = Response<Body>;
    type Error = InnerFailed;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, InnerFailed>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), InnerFailed>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let slot = req
            .extensions()
            .get::<ReservationSlot<Stamp>>()
            .cloned()
            .expect("the layer put a slot in the request");
        let stamped = req
            .headers()
            .get(STAMP_HEADER)
            .map(|value| value.to_str().unwrap().parse::<u32>().unwrap());
        if let Some(id) = stamped {
            slot.set(Stamp {
                id,
                callback_bound: self.callback_bound,
            });
        }
        let reply = self.reply;
        let events = Arc::clone(&self.events);
        let barrier = self.barrier.as_ref().map(Arc::clone);
        Box::pin(async move {
            if let Some(waiting) = barrier {
                waiting.wait().await;
            }
            let label = match stamped {
                Some(id) => id.to_string(),
                None => "none".to_owned(),
            };
            events
                .lock()
                .unwrap()
                .push(format!("handler {label} returned"));
            respond(reply)
        })
    }
}

fn respond(reply: Reply) -> Result<Response<Body>, InnerFailed> {
    match reply {
        Reply::Plain => Ok(Response::new(Body::from("ok"))),
        Reply::ServerError => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("upstream broke"))
            .unwrap()),
        Reply::Failed => Err(InnerFailed),
        Reply::Streaming => Ok(Response::new(Body::from_stream(stream::pending::<
            Result<Bytes, io::Error>,
        >()))),
    }
}

pub(super) struct NotReadyOnce {
    pub(super) inner: Handler,
    pub(super) polled: bool,
}

impl Service<Request<Body>> for NotReadyOnce {
    type Response = Response<Body>;
    type Error = InnerFailed;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, InnerFailed>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), InnerFailed>> {
        if self.polled {
            return self.inner.poll_ready(cx);
        }
        self.polled = true;
        Poll::Pending
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        self.inner.call(req)
    }
}

pub(super) fn events() -> Events {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) fn recorder(events: &Events, refuse: bool) -> Arc<Recorder> {
    Arc::new(Recorder {
        released: Mutex::new(Vec::new()),
        events: Arc::clone(events),
        refuse,
    })
}

pub(super) fn handler(reply: Reply, events: &Events) -> Handler {
    Handler {
        reply,
        events: Arc::clone(events),
        callback_bound: false,
        barrier: None,
    }
}

pub(super) fn request(stamp: Option<u32>) -> Request<Body> {
    let mut builder = Request::builder().uri("/v1/batches/batch_1/cancel");
    if let Some(id) = stamp {
        builder = builder.header(STAMP_HEADER, id.to_string());
    }
    builder.body(Body::empty()).unwrap()
}

pub(super) async fn text(response: Response<Body>) -> String {
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    str::from_utf8(&bytes).unwrap().to_owned()
}

pub(super) fn logged(records: &Arc<Mutex<String>>) -> String {
    records.lock().unwrap().clone()
}

#[derive(Clone)]
pub(super) struct CaptureWriter(pub(super) Arc<Mutex<String>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = str::from_utf8(buf)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        self.0
            .lock()
            .map_err(|source| io::Error::other(source.to_string()))?
            .push_str(text);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}
