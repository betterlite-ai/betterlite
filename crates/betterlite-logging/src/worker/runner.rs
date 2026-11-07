use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::future::select_all;
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, sleep_until, timeout};

use super::counters::Counters;
use super::timeouts::TimeoutSummary;
use super::{Callback, WorkerSettings};

pub(super) async fn run<C>(
    receiver: mpsc::Receiver<C>,
    settings: WorkerSettings,
    counters: Arc<Counters>,
    settled: Arc<Notify>,
) where
    C: Callback,
{
    let (timeouts, timeout_receiver) = mpsc::channel(settings.concurrency);
    let runner = Runner {
        receiver,
        timeout_receiver,
        timeouts,
        semaphore: Arc::new(Semaphore::new(settings.concurrency)),
        tasks: JoinSet::new(),
        counters,
        settled,
        summary: TimeoutSummary {
            window: settings.timeout_summary_window,
            burst: 0,
            last_callback: None,
            deadline: None,
        },
        event_timeout: settings.event_timeout,
    };
    runner.run().await;
}

enum Event<C> {
    Received(Option<C>),
    Joined(Option<Result<(), JoinError>>),
    TimedOut(Option<String>),
    Summarize,
}

type NextEvent<'runner, C> = Pin<Box<dyn Future<Output = Event<C>> + Send + 'runner>>;

struct Runner<C>
where
    C: Callback,
{
    receiver: mpsc::Receiver<C>,
    timeout_receiver: mpsc::Receiver<String>,
    timeouts: mpsc::Sender<String>,
    semaphore: Arc<Semaphore>,
    tasks: JoinSet<()>,
    counters: Arc<Counters>,
    settled: Arc<Notify>,
    summary: TimeoutSummary,
    event_timeout: Duration,
}

impl<C> Runner<C>
where
    C: Callback,
{
    async fn run(mut self) {
        loop {
            let event = self.next().await;
            match event {
                Event::Received(Some(callback)) => {
                    self.spawn(callback).await;
                    self.reap();
                }
                Event::Received(None) => break,
                Event::Joined(Some(joined)) => self.record_join(joined),
                Event::TimedOut(Some(callback)) => self.summary.record(callback),
                Event::Joined(None) | Event::TimedOut(None) => {}
                Event::Summarize => {
                    let total = self.counters.timed_out.load(Ordering::SeqCst);
                    self.summary.emit(self.event_timeout, total);
                }
            }
        }
        while let Some(joined) = self.tasks.join_next().await {
            self.record_join(joined);
        }
        while let Ok(callback) = self.timeout_receiver.try_recv() {
            self.summary.record(callback);
        }
        let total = self.counters.timed_out.load(Ordering::SeqCst);
        self.summary.emit(self.event_timeout, total);
    }

    async fn next(&mut self) -> Event<C> {
        let has_tasks = !self.tasks.is_empty();
        let deadline = self.summary.deadline.unwrap_or_else(Instant::now);
        let armed = self.summary.deadline.is_some();
        // tokio::select! expands a percent operator the denied restriction lint rejects, so select_all polls the same sources
        let mut pending: Vec<NextEvent<'_, C>> = Vec::new();
        pending.push(Box::pin(async {
            Event::Received(self.receiver.recv().await)
        }));
        if has_tasks {
            pending.push(Box::pin(async {
                Event::Joined(self.tasks.join_next().await)
            }));
        }
        pending.push(Box::pin(async {
            Event::TimedOut(self.timeout_receiver.recv().await)
        }));
        if armed {
            pending.push(Box::pin(async {
                sleep_until(deadline).await;
                Event::Summarize
            }));
        }
        let (event, _ready, remaining) = select_all(pending).await;
        drop(remaining);
        event
    }

    async fn spawn(&mut self, callback: C) {
        self.counters.dequeued.fetch_add(1, Ordering::SeqCst);
        let Ok(permit) = Arc::clone(&self.semaphore).acquire_owned().await else {
            self.counters.cancelled.fetch_add(1, Ordering::SeqCst);
            self.settled.notify_waiters();
            return;
        };
        let counters = Arc::clone(&self.counters);
        let settled = Arc::clone(&self.settled);
        let timeouts = self.timeouts.clone();
        let event_timeout = self.event_timeout;
        self.tasks.spawn(async move {
            let _permit = permit;
            counters.started.fetch_add(1, Ordering::SeqCst);
            run_callback(&counters, &settled, &timeouts, event_timeout, callback).await;
        });
    }

    fn reap(&mut self) {
        while let Some(joined) = self.tasks.try_join_next() {
            self.record_join(joined);
        }
    }

    fn record_join(&self, joined: Result<(), JoinError>) {
        let Err(error) = joined else {
            return;
        };
        tracing::error!(
            %error,
            "a logging callback task failed; the model response is unaffected"
        );
        self.counters.failed.fetch_add(1, Ordering::SeqCst);
        self.settled.notify_waiters();
    }
}

async fn run_callback<C>(
    counters: &Counters,
    settled: &Notify,
    timeouts: &mpsc::Sender<String>,
    event_timeout: Duration,
    callback: C,
) where
    C: Callback,
{
    let name = callback.name().to_owned();
    match timeout(event_timeout, callback.call()).await {
        Ok(Ok(())) => {
            counters.completed.fetch_add(1, Ordering::SeqCst);
        }
        Ok(Err(error)) => {
            tracing::warn!(
                callback = %name,
                %error,
                "the logging callback failed; the model response is unaffected"
            );
            counters.failed.fetch_add(1, Ordering::SeqCst);
        }
        Err(_) => {
            counters.timed_out.fetch_add(1, Ordering::SeqCst);
            send_timeout(timeouts, name);
        }
    }
    settled.notify_waiters();
}

fn send_timeout(timeouts: &mpsc::Sender<String>, name: String) {
    if timeouts.try_send(name).is_err() {
        tracing::debug!("the logging timeout summary is behind; the timeout is still counted");
    }
}
