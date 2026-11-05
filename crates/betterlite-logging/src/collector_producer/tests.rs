#![cfg(test)]

use super::{
    Fallback, FallbackError, ProducerError, PublishOutcome, SpendEventProducer,
    transport::Connector,
};
use betterlite_config::collector::{CollectorSettings, UnavailablePolicy};
use rust_decimal::Decimal;
use std::collections::VecDeque;
use std::future::{Future, pending, poll_fn, ready};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, duplex};
use tokio::sync::{Notify, oneshot};
use tokio::task::yield_now;
use tokio::time::{advance, timeout};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

async fn expect_io<Fut, Output>(read: Fut, message: &str) -> Output
where
    Fut: Future<Output = io::Result<Output>>,
{
    timeout(Duration::from_secs(5), read)
        .await
        .expect(message)
        .unwrap()
}

async fn close_producer(producer: &mut SpendEventProducer) {
    timeout(Duration::from_secs(5), producer.close())
        .await
        .expect("producer close hung");
}

async fn wait_for_tasks(tasks: &TaskTracker) {
    timeout(Duration::from_secs(5), tasks.wait())
        .await
        .expect("writer task never finished");
}

struct Controlled {
    sockets: Mutex<VecDeque<DuplexStream>>,
}
impl Connector for Arc<Controlled> {
    type Stream = DuplexStream;
    fn open(&self) -> impl Future<Output = io::Result<DuplexStream>> + Send {
        ready(
            self.sockets
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| io::Error::from(io::ErrorKind::ConnectionRefused)),
        )
    }
}
struct CountingRefusals {
    attempts: Arc<AtomicUsize>,
}
impl Connector for CountingRefusals {
    type Stream = DuplexStream;
    fn open(&self) -> impl Future<Output = io::Result<DuplexStream>> + Send {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        ready(Err(io::Error::from(io::ErrorKind::ConnectionRefused)))
    }
}
struct Harness {
    producer: SpendEventProducer,
    peer: DuplexStream,
    connect: Arc<Controlled>,
    events: Arc<Mutex<Vec<Vec<u8>>>>,
    tasks: TaskTracker,
    cancel: CancellationToken,
}
impl Harness {
    fn new(policy: UnavailablePolicy) -> Self {
        let (socket, peer) = duplex(8);
        let connect = Arc::new(Controlled {
            sockets: Mutex::new(VecDeque::from([socket])),
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let fallback: Fallback = Arc::new(move |line| {
            let output = Arc::clone(&captured);
            Box::pin(async move {
                output.lock().unwrap().push(line);
                Ok(())
            })
        });
        let tasks = TaskTracker::new();
        let settings = CollectorSettings {
            enabled: true,
            address: String::new(),
            buffer_size: 1,
            on_unavailable: policy,
            drain_timeout_seconds: Decimal::new(1, 1),
            connect_timeout_seconds: Decimal::ONE,
            job_role: None,
        };
        let producer =
            SpendEventProducer::start(&settings, fallback, &tasks, Arc::clone(&connect)).unwrap();
        Self {
            producer,
            peer,
            connect,
            events,
            tasks,
            cancel: CancellationToken::new(),
        }
    }
    async fn finish(&mut self) {
        close_producer(&mut self.producer).await;
        self.tasks.close();
        wait_for_tasks(&self.tasks).await;
    }
}

#[tokio::test(start_paused = true)]
async fn retains_fifo_and_never_replays_complete_lines() {
    let mut rig = Harness::new(UnavailablePolicy::Fallback);
    assert_eq!(
        rig.producer
            .publish(b"one\n".to_vec(), &rig.cancel)
            .await
            .unwrap(),
        PublishOutcome::Queued
    );
    let mut first = [0; 4];
    expect_io(rig.peer.read_exact(&mut first), "socket read timed out").await;
    assert_eq!(&first, b"one\n");
    assert_eq!(
        rig.producer
            .publish(b"two\n".to_vec(), &rig.cancel)
            .await
            .unwrap(),
        PublishOutcome::Queued
    );
    rig.finish().await;
    let mut last = Vec::new();
    expect_io(rig.peer.read_to_end(&mut last), "socket close timed out").await;
    assert_eq!(last, b"two\n");
    assert_eq!(rig.producer.stats().queued, 2);
    assert_eq!(rig.producer.stats().sent, 2);
    assert!(!rig.producer.stats().connected);
    assert!(rig.events.lock().unwrap().is_empty());
    rig.producer.close().await;
}

#[tokio::test(start_paused = true)]
async fn pressure_and_partial_close_apply_policy_once() {
    let mut rig = Harness::new(UnavailablePolicy::Fallback);
    let partial = vec![b'x'; 100];
    rig.producer
        .publish(partial.clone(), &rig.cancel)
        .await
        .unwrap();
    let mut prefix = [0; 1];
    expect_io(rig.peer.read_exact(&mut prefix), "socket read timed out").await;
    rig.producer
        .publish(b"queued\n".to_vec(), &rig.cancel)
        .await
        .unwrap();
    assert_eq!(
        rig.producer
            .publish(b"full\n".to_vec(), &rig.cancel)
            .await
            .unwrap(),
        PublishOutcome::Fallback
    );
    rig.finish().await;
    let mut remaining = Vec::new();
    expect_io(
        rig.peer.read_to_end(&mut remaining),
        "socket close timed out",
    )
    .await;
    assert!(remaining.len() < partial.len());
    assert_eq!(
        *rig.events.lock().unwrap(),
        [b"full\n".to_vec(), partial, b"queued\n".to_vec()]
    );
    assert_eq!(rig.producer.stats().sent, 0);
    assert_eq!(rig.producer.stats().fallback, 3);
}

#[tokio::test(start_paused = true)]
async fn half_close_reconnects_without_reusing_old_socket() {
    let mut rig = Harness::new(UnavailablePolicy::Fallback);
    rig.producer
        .publish(b"one\n".to_vec(), &rig.cancel)
        .await
        .unwrap();
    let mut bytes = [0; 4];
    expect_io(rig.peer.read_exact(&mut bytes), "socket read timed out").await;
    rig.peer.shutdown().await.unwrap();
    assert!(rig.producer.stats().connected);
    let (socket, mut next_peer) = duplex(8);
    rig.connect.sockets.lock().unwrap().push_back(socket);
    rig.producer
        .publish(b"two\n".to_vec(), &rig.cancel)
        .await
        .unwrap();
    expect_io(next_peer.read_exact(&mut bytes), "socket read timed out").await;
    assert_eq!(&bytes, b"two\n");
    rig.finish().await;
    assert!(rig.events.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn reconnect_backoff_uses_injected_tokio_time() {
    let mut rig = Harness::new(UnavailablePolicy::Drop);
    rig.connect.sockets.lock().unwrap().clear();
    rig.producer
        .publish(b"lost\n".to_vec(), &rig.cancel)
        .await
        .unwrap();
    yield_now().await;
    assert_eq!(
        rig.producer
            .publish(b"backoff\n".to_vec(), &rig.cancel)
            .await
            .unwrap(),
        PublishOutcome::Dropped
    );
    assert_eq!(rig.producer.stats().dropped, 2);
    advance(Duration::from_secs(1)).await;
    let (socket, mut peer) = duplex(8);
    rig.connect.sockets.lock().unwrap().push_back(socket);
    assert_eq!(
        rig.producer
            .publish(b"retry\n".to_vec(), &rig.cancel)
            .await
            .unwrap(),
        PublishOutcome::Queued
    );
    let mut received = [0; 6];
    expect_io(peer.read_exact(&mut received), "socket read timed out").await;
    assert_eq!(&received, b"retry\n");
    rig.finish().await;
}

#[tokio::test(start_paused = true)]
async fn backoff_window_skips_an_already_queued_reconnect_attempt() {
    let settings = CollectorSettings {
        enabled: true,
        address: String::new(),
        buffer_size: 2,
        on_unavailable: UnavailablePolicy::Drop,
        drain_timeout_seconds: Decimal::ONE,
        connect_timeout_seconds: Decimal::ONE,
        job_role: None,
    };
    let tasks = TaskTracker::new();
    let cancel = CancellationToken::new();
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut producer = SpendEventProducer::start(
        &settings,
        Arc::new(|_| Box::pin(async { Ok(()) })),
        &tasks,
        CountingRefusals {
            attempts: Arc::clone(&attempts),
        },
    )
    .unwrap();
    producer.publish(b"one".to_vec(), &cancel).await.unwrap();
    producer.publish(b"two".to_vec(), &cancel).await.unwrap();
    let mut polls_left = 10_000_u32;
    while producer.stats().dropped < 2 {
        polls_left -= 1;
        assert!(polls_left > 0, "dropped count never reached 2");
        yield_now().await;
    }
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    close_producer(&mut producer).await;
    tasks.close();
    wait_for_tasks(&tasks).await;
}

#[tokio::test(start_paused = true)]
async fn dropping_the_producer_lets_the_writer_task_finish() {
    let settings = CollectorSettings {
        enabled: true,
        address: String::new(),
        buffer_size: 1,
        on_unavailable: UnavailablePolicy::Drop,
        drain_timeout_seconds: Decimal::ONE,
        connect_timeout_seconds: Decimal::ONE,
        job_role: None,
    };
    let tasks = TaskTracker::new();
    let connect = Arc::new(Controlled {
        sockets: Mutex::new(VecDeque::new()),
    });
    let producer = SpendEventProducer::start(
        &settings,
        Arc::new(|_| Box::pin(async { Ok(()) })),
        &tasks,
        connect,
    )
    .unwrap();
    drop(producer);
    tasks.close();
    wait_for_tasks(&tasks).await;
}

#[tokio::test(start_paused = true)]
async fn fallback_completes_before_cancellation_propagates() {
    let mut rig = Harness::new(UnavailablePolicy::Fallback);
    rig.finish().await;
    rig.cancel.cancel();
    assert!(matches!(
        rig.producer
            .publish(b"cancelled\n".to_vec(), &rig.cancel)
            .await,
        Err(ProducerError::Cancelled)
    ));
    assert_eq!(*rig.events.lock().unwrap(), [b"cancelled\n".to_vec()]);
}

#[tokio::test(start_paused = true)]
async fn dropped_request_keeps_fallback_owned_and_contains_failure() {
    let mut rig = Harness::new(UnavailablePolicy::Fallback);
    rig.finish().await;
    let release = Arc::new(Notify::new());
    let trigger = Arc::clone(&release);
    let (finished, completed) = oneshot::channel();
    let completion = Arc::new(Mutex::new(Some(finished)));
    Arc::get_mut(&mut rig.producer.outage).unwrap().fallback = Arc::new(move |_| {
        let released = Arc::clone(&trigger);
        let output = Arc::clone(&completion);
        Box::pin(async move {
            released.notified().await;
            output.lock().unwrap().take().unwrap().send(()).unwrap();
            Err(FallbackError::Callback {
                source: io::Error::from(io::ErrorKind::Other),
            })
        })
    });
    let mut publish = Box::pin(rig.producer.publish(b"event".to_vec(), &rig.cancel));
    assert!(poll_fn(|context| Poll::Ready(publish.as_mut().poll(context).is_pending())).await);
    drop(publish);
    assert_eq!(rig.tasks.len(), 1);
    release.notify_one();
    completed.await.unwrap();
    rig.tasks.wait().await;
    assert_eq!(rig.producer.stats().fallback, 1);
}

#[rstest::rstest]
#[case::disabled(false, None, "tcp://localhost:bad", false)]
#[case::collector(true, Some("collector"), "tcp://localhost:bad", false)]
#[case::semantic(true, None, "tcp://example.com:1", false)]
#[case::syntax(true, None, "tcp://localhost:bad", true)]
#[tokio::test]
async fn construction_preserves_error_boundaries(
    #[case] enabled: bool,
    #[case] role: Option<&str>,
    #[case] address: &str,
    #[case] syntax: bool,
) {
    let settings = CollectorSettings {
        enabled,
        address: address.to_owned(),
        buffer_size: 1,
        on_unavailable: UnavailablePolicy::Fallback,
        drain_timeout_seconds: Decimal::ONE,
        connect_timeout_seconds: Decimal::ONE,
        job_role: role.map(str::to_owned),
    };
    let tasks = TaskTracker::new();
    let result =
        SpendEventProducer::build(&settings, Arc::new(|_| Box::pin(async { Ok(()) })), &tasks);
    assert_eq!(result.is_err(), syntax);
    if let Ok(producer) = result {
        assert!(producer.is_none());
    }
}

#[tokio::test(start_paused = true)]
async fn broken_partial_write_disconnects_and_enters_backoff() {
    let mut rig = Harness::new(UnavailablePolicy::Fallback);
    rig.producer
        .publish(vec![b'x'; 100], &rig.cancel)
        .await
        .unwrap();
    let mut prefix = [0];
    expect_io(rig.peer.read_exact(&mut prefix), "socket read timed out").await;
    drop(rig.peer);
    yield_now().await;
    assert_eq!(
        rig.producer
            .publish(b"backoff".to_vec(), &rig.cancel)
            .await
            .unwrap(),
        PublishOutcome::Fallback
    );
    close_producer(&mut rig.producer).await;
    rig.tasks.close();
    wait_for_tasks(&rig.tasks).await;
    assert_eq!(rig.producer.stats().sent, 0);
    assert_eq!(rig.producer.stats().fallback, 2);
    assert!(!rig.producer.stats().connected);
    assert_eq!(
        *rig.events.lock().unwrap(),
        [vec![b'x'; 100], b"backoff".to_vec()]
    );
}

struct Pending;
impl Connector for Pending {
    type Stream = DuplexStream;
    async fn open(&self) -> io::Result<DuplexStream> {
        pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn connect_timeout_applies_drop_and_close_stops_admission() {
    let settings = CollectorSettings {
        enabled: true,
        address: String::new(),
        buffer_size: 1,
        on_unavailable: UnavailablePolicy::Drop,
        drain_timeout_seconds: Decimal::TEN,
        connect_timeout_seconds: Decimal::ONE,
        job_role: None,
    };
    let tasks = TaskTracker::new();
    let cancel = CancellationToken::new();
    let mut producer = SpendEventProducer::start(
        &settings,
        Arc::new(|_| Box::pin(async { Ok(()) })),
        &tasks,
        Pending,
    )
    .unwrap();
    producer
        .publish(b"timeout".to_vec(), &cancel)
        .await
        .unwrap();
    close_producer(&mut producer).await;
    assert_eq!(producer.stats().dropped, 1);
    assert_eq!(
        producer.publish(b"closed".to_vec(), &cancel).await.unwrap(),
        PublishOutcome::Dropped
    );
    tasks.close();
    wait_for_tasks(&tasks).await;
    assert_eq!(producer.stats().dropped, 2);
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);
impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn drop_policy_logs_only_milestones() {
    use tracing::instrument::WithSubscriber as _;
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let mut rig = Harness::new(UnavailablePolicy::Drop);
    rig.finish().await;
    let dispatch = tracing::Dispatch::new(subscriber);
    let mut publishes_left = 10_000_u32;
    while rig.producer.stats().dropped < 2001 {
        publishes_left -= 1;
        assert!(publishes_left > 0, "dropped count never reached 2001");
        rig.producer
            .publish(Vec::new(), &rig.cancel)
            .with_subscriber(dispatch.clone())
            .await
            .unwrap();
    }
    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logs.lines().count(), 3);
    assert!(logs.contains("dropped=1\n"));
    assert!(logs.contains("dropped=1001\n"));
    assert!(logs.contains("dropped=2001\n"));
}

#[test]
fn spend_logging_preserves_supplied_or_active_trace() {
    use crate::spend_error::log_spend_error;
    use betterlite_config::collector::TracebackPolicy;
    use tokio::sync::watch;
    use tracing::subscriber::with_default;
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || writer.clone())
        .finish();
    let (sender, settings) = watch::channel(TracebackPolicy { suppress: true });
    with_default(subscriber, || {
        log_spend_error(&settings, "suppressed", Some("hidden"), None);
        sender.send_replace(TracebackPolicy { suppress: false });
        log_spend_error(&settings, "explicit", Some("supplied"), Some("unused"));
        log_spend_error(&settings, "implicit", None, Some("active"));
    });
    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.contains("hidden"));
    assert!(!logs.contains("unused"));
    assert!(logs.contains("supplied"));
    assert!(logs.contains("active"));
}

#[tokio::test]
async fn tcp_does_not_wait_for_collector_response() {
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let settings = CollectorSettings {
        enabled: true,
        address: format!("tcp://{}", listener.local_addr().unwrap()),
        buffer_size: 2,
        on_unavailable: UnavailablePolicy::Drop,
        drain_timeout_seconds: Decimal::ONE,
        connect_timeout_seconds: Decimal::ONE,
        job_role: None,
    };
    let tasks = TaskTracker::new();
    let cancel = CancellationToken::new();
    let mut producer =
        SpendEventProducer::build(&settings, Arc::new(|_| Box::pin(async { Ok(()) })), &tasks)
            .unwrap()
            .unwrap();
    producer
        .publish(b"first\n".to_vec(), &cancel)
        .await
        .unwrap();
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut bytes = [0; 6];
    expect_io(stream.read_exact(&mut bytes), "socket read timed out").await;
    producer
        .publish(b"other\n".to_vec(), &cancel)
        .await
        .unwrap();
    expect_io(stream.read_exact(&mut bytes), "socket read timed out").await;
    assert_eq!(&bytes, b"other\n");
    close_producer(&mut producer).await;
    tasks.close();
    wait_for_tasks(&tasks).await;
    assert_eq!(producer.stats().sent, 2);
    assert_eq!(producer.stats().dropped, 0);
}
