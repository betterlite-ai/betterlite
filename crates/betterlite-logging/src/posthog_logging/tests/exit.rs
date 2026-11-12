use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::batch::logger::BatchQueue;
use crate::posthog_logging::delivery::{PostFailure, PostRequest, Transport};
use crate::posthog_logging::sink::PostHogSink;
use crate::posthog_logging::tests::support::{
    AllowAllUrlCheck, Reply, ScriptedTransport, default_settings, field, fixed_id, logger_with,
    logger_with_shutdown, payload, queued, queued_for, settle, text,
};

struct GateTransport {
    inner: Arc<ScriptedTransport>,
    entered: Notify,
    release: Notify,
    gated: AtomicBool,
}

#[async_trait]
impl Transport for GateTransport {
    async fn post(&self, request: PostRequest) -> Result<u16, PostFailure> {
        if !self.gated.swap(true, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.post(request).await
    }
}

#[tokio::test]
async fn posthog_exit_flush_posts_each_credential_group() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, _tasks) = logger_with(default_settings(), &transport, fixed_id);
    let _first = logger
        .sink
        .queue
        .enqueue(queued_for("a-key", "https://a.example/", &payload()));
    let _second = logger
        .sink
        .queue
        .enqueue(queued_for("b-key", "https://b.example", &payload()));
    logger.sink.flush_on_exit().await;
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url, "https://a.example/batch/");
    assert_eq!(field(&requests[0].body, "api_key"), &text("a-key"));
    assert_eq!(requests[1].url, "https://b.example/batch/");
    assert_eq!(field(&requests[1].body, "api_key"), &text("b-key"));
    assert_eq!(logger.sink.queued_depth(), 0);
}

#[tokio::test]
async fn posthog_exit_flush_keeps_the_queue_when_a_post_fails() {
    let transport = ScriptedTransport::new(&[Reply::status(500)]);
    let (logger, _tasks) = logger_with(default_settings(), &transport, fixed_id);
    let _first = logger
        .sink
        .queue
        .enqueue(queued_for("a-key", "https://a.example", &payload()));
    let _second = logger
        .sink
        .queue
        .enqueue(queued_for("b-key", "https://b.example", &payload()));
    logger.sink.flush_on_exit().await;
    assert_eq!(transport.urls(), vec!["https://a.example/batch/"]);
    assert_eq!(logger.sink.queued_depth(), 2);
}

#[tokio::test]
async fn posthog_exit_flush_keeps_the_queue_on_a_transport_failure() {
    let transport = ScriptedTransport::new(&[Reply::network()]);
    let (logger, _tasks) = logger_with(default_settings(), &transport, fixed_id);
    let _first = logger.sink.queue.enqueue(queued(&payload()));
    logger.sink.flush_on_exit().await;
    assert_eq!(logger.sink.queued_depth(), 1);
}

#[tokio::test]
async fn posthog_exit_flush_sends_every_group_on_a_redirect_status() {
    let transport = ScriptedTransport::new(&[Reply::status(302), Reply::status(302)]);
    let (logger, _tasks) = logger_with(default_settings(), &transport, fixed_id);
    let _first = logger
        .sink
        .queue
        .enqueue(queued_for("a-key", "https://a.example", &payload()));
    let _second = logger
        .sink
        .queue
        .enqueue(queued_for("b-key", "https://b.example", &payload()));
    logger.sink.flush_on_exit().await;
    assert_eq!(transport.urls().len(), 2);
    assert_eq!(logger.sink.queued_depth(), 0);
}

#[tokio::test]
async fn posthog_exit_flush_with_an_empty_queue_sends_nothing() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, _tasks) = logger_with(default_settings(), &transport, fixed_id);
    logger.sink.flush_on_exit().await;
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn posthog_exit_flush_runs_when_the_shutdown_token_fires() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks, shutdown) = logger_with_shutdown(default_settings(), &transport, fixed_id);
    let _first = logger
        .sink
        .queue
        .enqueue(queued_for("a-key", "https://a.example", &payload()));
    let _second = logger
        .sink
        .queue
        .enqueue(queued_for("b-key", "https://b.example", &payload()));
    shutdown.cancel();
    settle(&tasks).await;
    assert_eq!(transport.urls().len(), 2);
    assert_eq!(logger.sink.queued_depth(), 0);
}

#[tokio::test]
async fn posthog_exit_flush_keeps_the_queue_when_the_shutdown_flush_fails() {
    let transport = ScriptedTransport::new(&[Reply::status(503)]);
    let (logger, tasks, shutdown) = logger_with_shutdown(default_settings(), &transport, fixed_id);
    let _first = logger.sink.queue.enqueue(queued(&payload()));
    shutdown.cancel();
    settle(&tasks).await;
    assert_eq!(transport.urls().len(), 1);
    assert_eq!(logger.sink.queued_depth(), 1);
}

#[tokio::test]
async fn posthog_exit_flush_never_resends_or_drops_an_event_enqueued_during_the_send() {
    let settings = default_settings();
    let queue = BatchQueue::new(settings.batch);
    let inner = ScriptedTransport::new(&[]);
    let gate = Arc::new(GateTransport {
        inner: Arc::clone(&inner),
        entered: Notify::new(),
        release: Notify::new(),
        gated: AtomicBool::new(false),
    });
    let concrete: Arc<GateTransport> = Arc::clone(&gate);
    let transport: Arc<dyn Transport> = concrete;
    let sink = PostHogSink {
        queue: queue.clone(),
        transport,
        url_check: Arc::new(AllowAllUrlCheck),
        mock_mode: false,
    };
    let _first = queue.enqueue(queued(&payload()));

    let mut tasks = JoinSet::new();
    let exit_sink = sink.clone();
    tasks.spawn(async move { exit_sink.flush_on_exit().await });

    gate.entered.notified().await;
    let _second = queue.enqueue(queued_for("new-key", "https://new.example", &payload()));
    let flush_sink = sink.clone();
    tasks.spawn(async move { flush_sink.flush().await });
    gate.release.notify_one();
    while let Some(result) = tasks.join_next().await {
        result.expect("the flush task finishes");
    }

    let requests = inner.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url, "https://us.i.posthog.com/batch/");
    assert_eq!(requests[1].url, "https://new.example/batch/");
    assert_eq!(field(&requests[1].body, "api_key"), &text("new-key"));
    assert_eq!(sink.queued_depth(), 0);
}
