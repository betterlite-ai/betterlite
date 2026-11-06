#![cfg(test)]

use std::future::{Future, pending};
use std::io::{Result as IoResult, Write};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Waker};
use std::time::Duration;

use rstest::rstest;
use tokio::sync::Notify;
use tokio::time::timeout;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use super::{BatchQueue, BatchSettings, Enqueued, FlushReport};
use crate::batch::delivery::{
    BatchCancelled, BatchDelivery, BatchResponse, BatchTransport, DeliverySettings, SendFailure,
    send_batch_with_413_split,
};
use crate::batch::retry::RequeuePolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reply {
    Delivered,
    Failed,
    Partial { kept: usize },
    Cancelled { kept: usize },
}

#[derive(Debug, Clone, Copy)]
enum FlushStep {
    Enqueue(&'static [&'static str]),
    Flush {
        reply: Reply,
        arrivals: &'static [&'static str],
    },
    Abort {
        arrivals: &'static [&'static str],
        settled: usize,
    },
    Overlap {
        second_events: &'static [&'static str],
    },
    SplitTimeout,
}

#[derive(Debug, Clone, Copy)]
struct FlushCase {
    settings: BatchSettings,
    preserve: bool,
    steps: &'static [FlushStep],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReportExpectation {
    batch: usize,
    delivered: usize,
    requeued: usize,
    dropped_oldest: usize,
    cancelled: bool,
}

#[derive(Debug, Clone, Copy)]
struct Expectation {
    sent: &'static [&'static [&'static str]],
    delivered: &'static [&'static str],
    queue: &'static [&'static str],
    enqueued: &'static [Enqueued],
    reports: &'static [ReportExpectation],
    warns: bool,
}

#[derive(Clone)]
enum Gate {
    Scripted(Reply),
    Held(Arc<AtomicBool>, Arc<Notify>),
    Pending,
}

struct TestDelivery {
    gate: Gate,
    settled_prefix: usize,
    attempts: Mutex<Vec<Vec<String>>>,
    delivered: Mutex<Vec<String>>,
    queue: BatchQueue<String>,
    arrivals: Vec<String>,
}

impl BatchDelivery<String> for TestDelivery {
    fn deliver(
        &self,
        batch: &[String],
        settled: &AtomicUsize,
    ) -> impl Future<Output = Result<Vec<String>, BatchCancelled<String>>> + Send {
        let accepted = match &self.gate {
            Gate::Scripted(Reply::Delivered) => batch.len(),
            Gate::Scripted(Reply::Partial { kept } | Reply::Cancelled { kept }) => {
                (*kept).min(batch.len())
            }
            Gate::Scripted(Reply::Failed) | Gate::Held(..) | Gate::Pending => 0,
        };
        self.delivered
            .lock()
            .unwrap()
            .extend(batch.iter().take(accepted).cloned());
        self.attempts.lock().unwrap().push(batch.to_vec());
        settled.fetch_add(self.settled_prefix.min(batch.len()), Ordering::SeqCst);
        for event in &self.arrivals {
            let _enqueued = self.queue.enqueue(event.clone());
        }
        let gate = self.gate.clone();
        let mut remaining = batch.to_vec();
        async move {
            match gate {
                Gate::Scripted(Reply::Delivered) => Ok(Vec::new()),
                Gate::Scripted(Reply::Failed) => Ok(remaining),
                Gate::Scripted(Reply::Partial { kept }) => Ok(remaining.split_off(kept)),
                Gate::Scripted(Reply::Cancelled { kept }) => {
                    let undelivered = remaining.split_off(kept);
                    Err(BatchCancelled { undelivered })
                }
                Gate::Held(entered, release) => {
                    entered.store(true, Ordering::SeqCst);
                    release.notified().await;
                    Ok(remaining)
                }
                Gate::Pending => pending().await,
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the split transport failed")]
struct SplitFailure;

struct SplitTransport {
    calls: AtomicUsize,
    attempts: Mutex<Vec<Vec<String>>>,
}

impl BatchTransport<String> for SplitTransport {
    type SendError = SplitFailure;
    type LimitError = SplitFailure;

    fn send(
        &self,
        batch: &[String],
    ) -> impl Future<Output = Result<BatchResponse, SendFailure<Self::SendError>>> + Send {
        self.attempts.lock().unwrap().push(batch.to_vec());
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        async move {
            if first {
                Ok(BatchResponse {
                    status: 200,
                    detail: "the scripted provider accepted the left half".to_owned(),
                })
            } else {
                pending().await
            }
        }
    }

    fn exceeds_limits(&self, batch: &[String]) -> Result<bool, Self::LimitError> {
        Ok(batch.len() > 2)
    }
}

const SPLIT_SETTINGS: DeliverySettings<'static> = DeliverySettings {
    success_status_codes: &[200],
    policy: RequeuePolicy::Undelivered,
    integration_name: "BatchDelivery",
    drop_error_message: "batch delivery dropped a single event the destination rejected as too large",
};

struct SplitterDelivery {
    transport: SplitTransport,
}

impl BatchDelivery<String> for SplitterDelivery {
    async fn deliver(
        &self,
        batch: &[String],
        settled: &AtomicUsize,
    ) -> Result<Vec<String>, BatchCancelled<String>> {
        send_batch_with_413_split(&self.transport, batch, SPLIT_SETTINGS, settled).await
    }
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

struct Outcome {
    queue: Vec<String>,
    reports: Vec<FlushReport>,
    attempts: Vec<Vec<String>>,
    enqueued: Vec<Enqueued>,
    delivered: Vec<String>,
}

fn scripted(queue: &BatchQueue<String>, reply: Reply, arrivals: &[&str]) -> TestDelivery {
    TestDelivery {
        gate: Gate::Scripted(reply),
        settled_prefix: 0,
        attempts: Mutex::new(Vec::new()),
        delivered: Mutex::new(Vec::new()),
        queue: queue.clone(),
        arrivals: owned(arrivals),
    }
}

async fn flush_once(
    queue: &BatchQueue<String>,
    reply: Reply,
    arrivals: &[&str],
) -> (FlushReport, Vec<String>, Vec<String>) {
    let delivery = scripted(queue, reply, arrivals);
    let report = queue.flush(&delivery).await;
    let sent = delivery
        .attempts
        .lock()
        .unwrap()
        .first()
        .cloned()
        .unwrap_or_default();
    let delivered = delivery.delivered.lock().unwrap().clone();
    (report, sent, delivered)
}

async fn drive(case: &FlushCase) -> Outcome {
    let mut queue = BatchQueue::new(case.settings);
    queue.preserve_during_flush = case.preserve;
    let mut outcome = Outcome {
        queue: Vec::new(),
        reports: Vec::new(),
        attempts: Vec::new(),
        enqueued: Vec::new(),
        delivered: Vec::new(),
    };
    for step in case.steps {
        match step {
            FlushStep::Enqueue(events) => {
                for event in *events {
                    outcome.enqueued.push(queue.enqueue((*event).to_owned()));
                }
            }
            FlushStep::Flush { reply, arrivals } => {
                let (report, sent, delivered) = flush_once(&queue, *reply, arrivals).await;
                outcome.reports.push(report);
                outcome.attempts.push(sent);
                outcome.delivered.extend(delivered);
            }
            FlushStep::Abort { arrivals, settled } => {
                outcome
                    .attempts
                    .push(abort_flush(&queue, arrivals, *settled).await);
            }
            FlushStep::Overlap { second_events } => {
                let finished = overlap_flushes(&queue, second_events, &mut outcome.enqueued).await;
                outcome.attempts.push(finished.first);
                outcome.attempts.push(finished.second);
                outcome.reports.extend(finished.reports);
            }
            FlushStep::SplitTimeout => outcome.attempts.extend(split_timeout(&queue).await),
        }
    }
    outcome.queue = queue.pending();
    outcome
}

async fn split_timeout(queue: &BatchQueue<String>) -> Vec<Vec<String>> {
    let delivery = SplitterDelivery {
        transport: SplitTransport {
            calls: AtomicUsize::new(0),
            attempts: Mutex::new(Vec::new()),
        },
    };
    let waiting = timeout(Duration::from_millis(20), queue.flush(&delivery)).await;
    assert!(
        waiting.is_err(),
        "the pending right half should keep the flush waiting"
    );
    delivery.transport.attempts.lock().unwrap().clone()
}

async fn abort_flush(queue: &BatchQueue<String>, arrivals: &[&str], settled: usize) -> Vec<String> {
    let delivery = TestDelivery {
        gate: Gate::Pending,
        settled_prefix: settled,
        attempts: Mutex::new(Vec::new()),
        delivered: Mutex::new(Vec::new()),
        queue: queue.clone(),
        arrivals: owned(arrivals),
    };
    let waiting = timeout(Duration::from_millis(20), queue.flush(&delivery)).await;
    assert!(
        waiting.is_err(),
        "the pending delivery should keep the flush waiting"
    );
    first_attempt(&delivery.attempts)
}

fn poll_pending<F>(future: Pin<&mut F>) -> bool
where
    F: Future,
{
    let mut context = Context::from_waker(Waker::noop());
    future.poll(&mut context).is_pending()
}

fn first_attempt(attempts: &Mutex<Vec<Vec<String>>>) -> Vec<String> {
    attempts
        .lock()
        .unwrap()
        .first()
        .cloned()
        .unwrap_or_default()
}

struct Finished {
    first: Vec<String>,
    second: Vec<String>,
    reports: [FlushReport; 2],
}

async fn overlap_flushes(
    queue: &BatchQueue<String>,
    second_events: &[&str],
    enqueued: &mut Vec<Enqueued>,
) -> Finished {
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(Notify::new());
    let gated = TestDelivery {
        gate: Gate::Held(Arc::clone(&entered), Arc::clone(&release)),
        settled_prefix: 0,
        attempts: Mutex::new(Vec::new()),
        delivered: Mutex::new(Vec::new()),
        queue: queue.clone(),
        arrivals: Vec::new(),
    };
    let first = queue.flush(&gated);
    tokio::pin!(first);
    assert!(poll_pending(first.as_mut()), "the gated flush should wait");
    assert!(
        entered.load(Ordering::SeqCst),
        "the first delivery did not start"
    );
    for event in second_events {
        enqueued.push(queue.enqueue((*event).to_owned()));
    }
    let blocked = scripted(queue, Reply::Failed, &[]);
    let second = queue.flush(&blocked);
    tokio::pin!(second);
    assert!(
        poll_pending(second.as_mut()),
        "a second flush ran while one was in flight"
    );
    assert!(
        blocked.attempts.lock().unwrap().is_empty(),
        "the second delivery started while one was in flight"
    );
    release.notify_one();
    let reports = [first.await, second.await];
    Finished {
        first: first_attempt(&gated.attempts),
        second: first_attempt(&blocked.attempts),
        reports,
    }
}

fn owned(events: &[&str]) -> Vec<String> {
    events.iter().map(|event| (*event).to_owned()).collect()
}

const fn queued(depth: usize, dropped_oldest: usize) -> Enqueued {
    Enqueued {
        depth,
        dropped_oldest,
    }
}

const ENQUEUED_TWO: &[Enqueued] = &[queued(1, 0), queued(2, 0)];
const ENQUEUED_FOUR: &[Enqueued] = &[queued(1, 0), queued(2, 0), queued(3, 0), queued(4, 0)];
const ENQUEUED_THREE: &[Enqueued] = &[queued(1, 0), queued(2, 0), queued(3, 0)];
const ENQUEUED_OVERFLOW: &[Enqueued] = &[queued(1, 0), queued(2, 0), queued(3, 0), queued(3, 1)];
const ENQUEUED_AFTER_DRAIN: &[Enqueued] = &[queued(1, 0), queued(2, 0), queued(1, 0)];

fn report_expectation(report: &FlushReport) -> ReportExpectation {
    ReportExpectation {
        batch: report.batch,
        delivered: report.delivered,
        requeued: report.requeued,
        dropped_oldest: report.dropped_oldest,
        cancelled: report.cancelled,
    }
}

const DEFAULT: BatchSettings = BatchSettings {
    batch_size: 4,
    flush_interval: Duration::from_secs(5),
    max_queue_size: 100,
};

const SMALL: BatchSettings = BatchSettings {
    batch_size: 4,
    flush_interval: Duration::from_secs(5),
    max_queue_size: 3,
};

#[rstest]
#[case::empty_flush_sends_nothing(
    FlushCase { settings: DEFAULT, preserve: false, steps: &[FlushStep::Flush { reply: Reply::Delivered, arrivals: &[] }] },
    Expectation {
        sent: &[&[]],
        delivered: &[],
        queue: &[],
        enqueued: &[],
        reports: &[ReportExpectation { batch: 0, delivered: 0, requeued: 0, dropped_oldest: 0, cancelled: false }],
        warns: false,
    }
)]
#[case::delivered_flush_clears_the_queue(
    FlushCase {
        settings: DEFAULT,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b"]),
            FlushStep::Flush { reply: Reply::Delivered, arrivals: &[] },
        ],
    },
    Expectation {
        sent: &[&["a", "b"]],
        delivered: &["a", "b"],
        queue: &[],
        enqueued: ENQUEUED_TWO,
        reports: &[ReportExpectation { batch: 2, delivered: 2, requeued: 0, dropped_oldest: 0, cancelled: false }],
        warns: false,
    }
)]
#[case::failed_flush_retains_the_events(
    FlushCase {
        settings: DEFAULT,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b"]),
            FlushStep::Flush { reply: Reply::Failed, arrivals: &[] },
            FlushStep::Flush { reply: Reply::Delivered, arrivals: &[] },
        ],
    },
    Expectation {
        sent: &[&["a", "b"], &["a", "b"]],
        delivered: &["a", "b"],
        queue: &[],
        enqueued: ENQUEUED_TWO,
        reports: &[
            ReportExpectation { batch: 2, delivered: 0, requeued: 2, dropped_oldest: 0, cancelled: false },
            ReportExpectation { batch: 2, delivered: 2, requeued: 0, dropped_oldest: 0, cancelled: false },
        ],
        warns: false,
    }
)]
#[case::overflow_drops_the_oldest(
    FlushCase {
        settings: SMALL,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b"]),
            FlushStep::Flush { reply: Reply::Failed, arrivals: &["c", "d"] },
        ],
    },
    Expectation {
        sent: &[&["a", "b"]],
        delivered: &[],
        queue: &["b", "c", "d"],
        enqueued: ENQUEUED_TWO,
        reports: &[ReportExpectation { batch: 2, delivered: 0, requeued: 2, dropped_oldest: 1, cancelled: false }],
        warns: true,
    }
)]
#[case::enqueue_overflow_drops_the_oldest(
    FlushCase {
        settings: SMALL,
        preserve: false,
        steps: &[FlushStep::Enqueue(&["a", "b", "c", "d"])],
    },
    Expectation {
        sent: &[],
        delivered: &[],
        queue: &["b", "c", "d"],
        enqueued: ENQUEUED_OVERFLOW,
        reports: &[],
        warns: true,
    }
)]
#[case::aborted_flush_restores_the_batch(
    FlushCase {
        settings: DEFAULT,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b"]),
            FlushStep::Abort { arrivals: &["c"], settled: 0 },
        ],
    },
    Expectation {
        sent: &[&["a", "b"]],
        delivered: &[],
        queue: &["a", "b", "c"],
        enqueued: ENQUEUED_TWO,
        reports: &[],
        warns: false,
    }
)]
#[case::aborted_flush_restores_only_the_unconfirmed_remainder(
    FlushCase {
        settings: DEFAULT,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b", "c", "d"]),
            FlushStep::Abort { arrivals: &[], settled: 2 },
        ],
    },
    Expectation {
        sent: &[&["a", "b", "c", "d"]],
        delivered: &[],
        queue: &["c", "d"],
        enqueued: ENQUEUED_FOUR,
        reports: &[],
        warns: false,
    }
)]
#[case::split_flush_keeps_the_unaccepted_half(
    FlushCase {
        settings: DEFAULT,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b", "c", "d"]),
            FlushStep::SplitTimeout,
        ],
    },
    Expectation {
        sent: &[&["a", "b"], &["c", "d"]],
        delivered: &[],
        queue: &["c", "d"],
        enqueued: ENQUEUED_FOUR,
        reports: &[],
        warns: false,
    }
)]
#[case::overlapping_flushes_run_one_at_a_time(
    FlushCase {
        settings: DEFAULT,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b"]),
            FlushStep::Overlap { second_events: &["c"] },
        ],
    },
    Expectation {
        sent: &[&["a", "b"], &["a", "b", "c"]],
        delivered: &[],
        queue: &["a", "b", "c"],
        enqueued: ENQUEUED_AFTER_DRAIN,
        reports: &[
            ReportExpectation { batch: 2, delivered: 0, requeued: 2, dropped_oldest: 0, cancelled: false },
            ReportExpectation { batch: 3, delivered: 0, requeued: 3, dropped_oldest: 0, cancelled: false },
        ],
        warns: false,
    }
)]
#[case::undelivered_is_requeued_before_arrivals(
    FlushCase {
        settings: DEFAULT,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b"]),
            FlushStep::Flush { reply: Reply::Partial { kept: 1 }, arrivals: &["c"] },
        ],
    },
    Expectation {
        sent: &[&["a", "b"]],
        delivered: &["a"],
        queue: &["b", "c"],
        enqueued: ENQUEUED_TWO,
        reports: &[ReportExpectation { batch: 2, delivered: 1, requeued: 1, dropped_oldest: 0, cancelled: false }],
        warns: false,
    }
)]
#[case::cancelled_flush_keeps_the_undelivered(
    FlushCase {
        settings: DEFAULT,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b", "c"]),
            FlushStep::Flush { reply: Reply::Cancelled { kept: 1 }, arrivals: &["d"] },
        ],
    },
    Expectation {
        sent: &[&["a", "b", "c"]],
        delivered: &["a"],
        queue: &["b", "c", "d"],
        enqueued: ENQUEUED_THREE,
        reports: &[ReportExpectation { batch: 3, delivered: 1, requeued: 2, dropped_oldest: 0, cancelled: true }],
        warns: false,
    }
)]
#[case::preserve_keeps_events_added_during_the_flush(
    FlushCase {
        settings: DEFAULT,
        preserve: true,
        steps: &[
            FlushStep::Enqueue(&["a", "b"]),
            FlushStep::Flush { reply: Reply::Delivered, arrivals: &["c"] },
        ],
    },
    Expectation {
        sent: &[&["a", "b"]],
        delivered: &["a", "b"],
        queue: &["c"],
        enqueued: ENQUEUED_TWO,
        reports: &[ReportExpectation { batch: 2, delivered: 2, requeued: 0, dropped_oldest: 0, cancelled: false }],
        warns: false,
    }
)]
#[case::clearing_drops_events_added_during_the_flush(
    FlushCase {
        settings: DEFAULT,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b"]),
            FlushStep::Flush { reply: Reply::Delivered, arrivals: &["c"] },
        ],
    },
    Expectation {
        sent: &[&["a", "b"]],
        delivered: &["a", "b"],
        queue: &[],
        enqueued: ENQUEUED_TWO,
        reports: &[ReportExpectation { batch: 2, delivered: 2, requeued: 0, dropped_oldest: 0, cancelled: false }],
        warns: false,
    }
)]
#[case::requeue_keeps_the_newest_when_over_capacity(
    FlushCase {
        settings: SMALL,
        preserve: false,
        steps: &[
            FlushStep::Enqueue(&["a", "b", "c"]),
            FlushStep::Flush {
                reply: Reply::Partial { kept: 1 },
                arrivals: &["d", "e"],
            },
        ],
    },
    Expectation {
        sent: &[&["a", "b", "c"]],
        delivered: &["a"],
        queue: &["c", "d", "e"],
        enqueued: ENQUEUED_THREE,
        reports: &[ReportExpectation { batch: 3, delivered: 1, requeued: 2, dropped_oldest: 1, cancelled: false }],
        warns: true,
    }
)]
#[tokio::test]
async fn batch_delivery_behavior_tests_behavior(
    #[case] case: FlushCase,
    #[case] expected: Expectation,
) {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let _guard = set_default(subscriber);
    let outcome = drive(&case).await;
    let expected_sent: Vec<Vec<String>> =
        expected.sent.iter().map(|events| owned(events)).collect();
    let expected_reports: Vec<ReportExpectation> = expected.reports.to_vec();
    assert_eq!(outcome.attempts, expected_sent);
    assert_eq!(outcome.queue, owned(expected.queue));
    assert_eq!(outcome.enqueued, expected.enqueued);
    assert_eq!(outcome.delivered, owned(expected.delivered));
    assert_eq!(
        outcome
            .reports
            .iter()
            .map(report_expectation)
            .collect::<Vec<_>>(),
        expected_reports
    );
    assert_eq!(buffer.contents().contains("max_queue_size"), expected.warns);
}
