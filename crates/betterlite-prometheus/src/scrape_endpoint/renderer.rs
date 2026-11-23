use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use futures_util::FutureExt as _;
use futures_util::future::{BoxFuture, Shared};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use crate::collector_registry::CollectorRegistry;

use super::{ScrapeRenderError, ScrapeRequest, render_scrape};

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
    pub(crate) registry: Arc<CollectorRegistry>,
    pub(crate) state: Arc<Mutex<RenderState>>,
}

impl CoalescedScrapeRenderer {
    #[must_use]
    pub fn new(registry: Arc<CollectorRegistry>) -> Self {
        Self {
            registry,
            state: Arc::new(Mutex::new(RenderState {
                tasks: JoinSet::new(),
                inflight: HashMap::new(),
                next_id: 0,
            })),
        }
    }

    pub async fn render(&self, request: ScrapeRequest) -> Result<Bytes, ScrapeRenderError> {
        let shared = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            while let Some(outcome) = state.tasks.try_join_next() {
                if let Err(error) = outcome {
                    tracing::warn!(%error, "a scrape render task stopped without sending a body");
                }
            }
            let running = state
                .inflight
                .get(&request)
                .map(|inflight| SharedRender::clone(&inflight.render));
            match running {
                Some(shared) => shared,
                None => {
                    let id = state.next_id;
                    state.next_id = state.next_id.wrapping_add(1);
                    let (sender, receiver) = oneshot::channel();
                    let scheduled = request.clone();
                    let registry = Arc::clone(&self.registry);
                    let state_handle = Arc::clone(&self.state);
                    let _spawned = state.tasks.spawn_blocking(move || {
                        let rendered = render_scrape(&registry, &scheduled);
                        let mut finished =
                            state_handle.lock().unwrap_or_else(PoisonError::into_inner);
                        if finished
                            .inflight
                            .get(&scheduled)
                            .is_some_and(|inflight| inflight.id == id)
                        {
                            let _removed = finished.inflight.remove(&scheduled);
                        }
                        let _sent = sender.send(Ok(Bytes::from(rendered)));
                    });
                    let shared = shared_render(receiver);
                    let _previous = state.inflight.insert(
                        request,
                        InFlight {
                            id,
                            render: SharedRender::clone(&shared),
                        },
                    );
                    shared
                }
            }
        };
        shared.await
    }
}

// asyncio.shield: the blocking render keeps going when every waiter is dropped, and a task that sent nothing fails every waiter
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
