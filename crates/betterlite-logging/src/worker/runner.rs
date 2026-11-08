use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::future::{FutureExt as _, select_all};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::Queued;
use super::counters::Counters;
use super::timeouts::TimeoutBurst;
use super::{Callback, WorkerSettings};

pub(super) struct RunnerContext {
    pub(super) counters: Arc<Counters>,
    pub(super) settled: Arc<Notify>,
    pub(super) burst: Arc<TimeoutBurst>,
    pub(super) shutdown: CancellationToken,
    pub(super) summarizer: JoinHandle<()>,
}

pub(super) async fn run<C>(
    receiver: mpsc::Receiver<Queued<C>>,
    settings: WorkerSettings,
    context: RunnerContext,
) where
    C: Callback,
{
    let runner = Runner {
        receiver,
        semaphore: Arc::new(Semaphore::new(settings.concurrency)),
        tasks: JoinSet::new(),
        counters: context.counters,
        settled: context.settled,
        burst: context.burst,
        shutdown: context.shutdown,
        summarizer: Some(context.summarizer),
        event_timeout: settings.event_timeout,
    };
    runner.run().await;
}

enum Event<C> {
    Received(Option<Queued<C>>),
    Joined,
    Shutdown,
}

type NextEvent<'runner, C> = Pin<Box<dyn Future<Output = Event<C>> + Send + 'runner>>;
type PermitFuture = Pin<Box<dyn Future<Output = Option<OwnedSemaphorePermit>> + Send>>;

struct Runner<C>
where
    C: Callback,
{
    receiver: mpsc::Receiver<Queued<C>>,
    semaphore: Arc<Semaphore>,
    tasks: JoinSet<()>,
    counters: Arc<Counters>,
    settled: Arc<Notify>,
    burst: Arc<TimeoutBurst>,
    shutdown: CancellationToken,
    summarizer: Option<JoinHandle<()>>,
    event_timeout: Duration,
}

impl<C> Runner<C>
where
    C: Callback,
{
    async fn run(mut self) {
        let mut closed = false;
        loop {
            let event = self.next(closed).await;
            match event {
                Event::Received(Some(queued)) => self.spawn(queued).await,
                Event::Received(None) => closed = true,
                Event::Joined => {}
                Event::Shutdown => {
                    self.tasks.shutdown().await;
                    self.finish().await;
                    return;
                }
            }
            while self.tasks.try_join_next().is_some() {}
            if closed && self.tasks.is_empty() {
                break;
            }
        }
        self.finish().await;
    }

    async fn next(&mut self, closed: bool) -> Event<C> {
        if self.shutdown.is_cancelled() {
            return Event::Shutdown;
        }
        let has_tasks = !self.tasks.is_empty();
        let mut pending: Vec<NextEvent<'_, C>> = Vec::new();
        if !closed {
            pending.push(Box::pin(async {
                Event::Received(self.receiver.recv().await)
            }));
        }
        if has_tasks {
            pending.push(Box::pin(async {
                drop(self.tasks.join_next().await);
                Event::Joined
            }));
        }
        let shutdown = self.shutdown.clone();
        pending.push(Box::pin(async move {
            shutdown.cancelled().await;
            Event::Shutdown
        }));
        let (event, _ready, remaining) = select_all(pending).await;
        drop(remaining);
        event
    }

    async fn acquire_permit(&self) -> Option<OwnedSemaphorePermit> {
        if self.shutdown.is_cancelled() {
            return None;
        }
        let semaphore = Arc::clone(&self.semaphore);
        let granted = async move { semaphore.acquire_owned().await.ok() };
        let shutdown = self.shutdown.clone();
        let stopped = async move {
            shutdown.cancelled().await;
            None
        };
        let mut pending: Vec<PermitFuture> = Vec::new();
        pending.push(Box::pin(granted));
        pending.push(Box::pin(stopped));
        select_all(pending).await.0
    }

    async fn spawn(&mut self, queued: Queued<C>) {
        self.counters.dequeued.fetch_add(1, Ordering::SeqCst);
        let Some(permit) = self.acquire_permit().await else {
            self.counters.cancelled.fetch_add(1, Ordering::SeqCst);
            self.settled.notify_waiters();
            return;
        };
        let Queued { span, callback } = queued;
        let counters = Arc::clone(&self.counters);
        let settled = Arc::clone(&self.settled);
        let burst = Arc::clone(&self.burst);
        let event_timeout = self.event_timeout;
        self.tasks.spawn(
            async move {
                let _permit = permit;
                counters.started.fetch_add(1, Ordering::SeqCst);
                run_callback(&counters, &settled, &burst, event_timeout, callback).await;
            }
            .instrument(span),
        );
    }

    async fn finish(&mut self) {
        self.shutdown.cancel();
        let Some(summarizer) = self.summarizer.take() else {
            return;
        };
        if summarizer.await.is_err() {
            tracing::error!("the logging timeout summarizer failed");
        }
    }
}

async fn run_callback<C>(
    counters: &Counters,
    settled: &Notify,
    burst: &TimeoutBurst,
    event_timeout: Duration,
    callback: C,
) where
    C: Callback,
{
    let name = callback.name().to_owned();
    let outcome = timeout(
        event_timeout,
        AssertUnwindSafe(callback.call()).catch_unwind(),
    )
    .await;
    match outcome {
        Ok(Ok(Ok(()))) => {
            counters.completed.fetch_add(1, Ordering::SeqCst);
        }
        Ok(Ok(Err(error))) => {
            tracing::warn!(
                callback = %name,
                %error,
                "the logging callback failed; the model response is unaffected"
            );
            counters.failed.fetch_add(1, Ordering::SeqCst);
        }
        Ok(Err(_panic)) => {
            tracing::error!(
                callback = %name,
                "a logging callback task failed; the model response is unaffected"
            );
            counters.failed.fetch_add(1, Ordering::SeqCst);
        }
        Err(_) => {
            counters.timed_out.fetch_add(1, Ordering::SeqCst);
            burst.record(name);
        }
    }
    settled.notify_waiters();
}
