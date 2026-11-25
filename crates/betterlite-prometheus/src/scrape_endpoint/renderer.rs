use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use futures_util::FutureExt as _;
use futures_util::future::{BoxFuture, Shared};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use super::{ScrapeRenderError, ScrapeRequest, ScrapeSource};

pub(crate) type SharedRender = Shared<BoxFuture<'static, Result<Bytes, ScrapeRenderError>>>;

pub(crate) struct InFlight {
    pub(crate) id: u64,
    pub(crate) render: SharedRender,
}

pub(crate) struct RenderState {
    pub(crate) tasks: JoinSet<()>,
    pub(crate) inflight: HashMap<ScrapeRequest, InFlight>,
    pub(crate) next_id: u64,
}

// LiteLLM's CoalescedScrapeRenderer: one render off the event loop, shared by every scrape that asks for the same output
pub struct CoalescedScrapeRenderer {
    pub(crate) source: Arc<ScrapeSource>,
    pub(crate) state: Arc<Mutex<RenderState>>,
}

impl CoalescedScrapeRenderer {
    #[must_use]
    pub fn new(source: ScrapeSource) -> Self {
        Self {
            source: Arc::new(source),
            state: Arc::new(Mutex::new(RenderState {
                tasks: JoinSet::new(),
                inflight: HashMap::new(),
                next_id: 0,
            })),
        }
    }

    pub async fn render(&self, request: ScrapeRequest) -> Result<Bytes, ScrapeRenderError> {
        let running = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            poll_finished(&mut state.tasks);
            let pending = running_render(&mut state, &self.source, &self.state, &request);
            drop(state);
            pending
        };
        running.await
    }
}

// asyncio.shield: the render keeps going when every waiter is dropped, and a task that sent nothing fails every waiter
pub(crate) fn shared_render(
    receiver: oneshot::Receiver<Result<Bytes, ScrapeRenderError>>,
) -> SharedRender {
    async move {
        match receiver.await {
            Ok(result) => result,
            Err(_stopped) => Err(ScrapeRenderError::RenderTaskStopped),
        }
    }
    .boxed()
    .shared()
}

fn running_render(
    state: &mut RenderState,
    source: &Arc<ScrapeSource>,
    shared_state: &Arc<Mutex<RenderState>>,
    request: &ScrapeRequest,
) -> SharedRender {
    if let Some(inflight) = state.inflight.get(request) {
        return SharedRender::clone(&inflight.render);
    }
    start_render(state, source, shared_state, request)
}

fn start_render(
    state: &mut RenderState,
    source: &Arc<ScrapeSource>,
    shared_state: &Arc<Mutex<RenderState>>,
    request: &ScrapeRequest,
) -> SharedRender {
    let id = state.next_id;
    state.next_id = state.next_id.wrapping_add(1);
    let (sender, receiver) = oneshot::channel();
    let scheduled = request.clone();
    let worker_source = Arc::clone(source);
    let worker_state = Arc::clone(shared_state);
    let _spawned = state.tasks.spawn_blocking(move || {
        finish_render(&worker_source, &scheduled, &worker_state, sender, id);
    });
    let shared = shared_render(receiver);
    let _previous = state.inflight.insert(
        request.clone(),
        InFlight {
            id,
            render: SharedRender::clone(&shared),
        },
    );
    shared
}

// the entry goes before the waiters hear back, and only while it still names this render
fn finish_render(
    source: &ScrapeSource,
    request: &ScrapeRequest,
    shared_state: &Mutex<RenderState>,
    sender: oneshot::Sender<Result<Bytes, ScrapeRenderError>>,
    id: u64,
) {
    let rendered = source.render(request);
    let mut state = shared_state.lock().unwrap_or_else(PoisonError::into_inner);
    if state
        .inflight
        .get(request)
        .is_some_and(|inflight| inflight.id == id)
    {
        let _removed = state.inflight.remove(request);
    }
    drop(state);
    let _sent = sender.send(rendered.map(Bytes::from));
}

pub(crate) fn poll_finished(tasks: &mut JoinSet<()>) {
    while let Some(outcome) = tasks.try_join_next() {
        if let Err(error) = outcome {
            tracing::warn!(%error, "a scrape render task stopped without sending a body");
        }
    }
}
