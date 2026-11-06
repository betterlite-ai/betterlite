#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rstest::rstest;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use super::{BatchQueue, BatchSettings, FlushReport};
use crate::batch::delivery::{BatchCancelled, BatchDelivery};

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
    depths: &'static [usize],
    reports: &'static [ReportExpectation],
    warns: bool,
}

struct ScriptedDelivery {
    replies: Mutex<VecDeque<Reply>>,
    attempts: Mutex<Vec<Vec<String>>>,
    delivered: Mutex<Vec<String>>,
    queue: BatchQueue<String>,
    arrivals: Vec<String>,
}

impl BatchDelivery<String> for ScriptedDelivery {
    fn deliver(
        &self,
        batch: &[String],
    ) -> impl Future<Output = Result<Vec<String>, BatchCancelled<String>>> + Send {
        let reply = self.replies.lock().unwrap().pop_front().unwrap();
        let accepted = match reply {
            Reply::Delivered => batch.len(),
            Reply::Failed => 0,
            Reply::Partial { kept } | Reply::Cancelled { kept } => kept.min(batch.len()),
        };
        self.delivered
            .lock()
            .unwrap()
            .extend(batch.iter().take(accepted).cloned());
        self.attempts.lock().unwrap().push(batch.to_vec());
        for event in &self.arrivals {
            let _depth = self.queue.enqueue(event.clone());
        }
        let mut remaining = batch.to_vec();
        async move {
            match reply {
                Reply::Delivered => Ok(Vec::new()),
                Reply::Failed => Ok(remaining),
                Reply::Partial { kept } => Ok(remaining.split_off(kept)),
                Reply::Cancelled { kept } => {
                    let undelivered = remaining.split_off(kept);
                    Err(BatchCancelled { undelivered })
                }
            }
        }
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
    depths: Vec<usize>,
    delivered: Vec<String>,
}

async fn flush_once(
    queue: &BatchQueue<String>,
    reply: Reply,
    arrivals: &[&str],
) -> (FlushReport, Vec<String>, Vec<String>) {
    let delivery = ScriptedDelivery {
        replies: Mutex::new(VecDeque::from([reply])),
        attempts: Mutex::new(Vec::new()),
        delivered: Mutex::new(Vec::new()),
        queue: queue.clone(),
        arrivals: owned(arrivals),
    };
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
        depths: Vec::new(),
        delivered: Vec::new(),
    };
    for step in case.steps {
        match step {
            FlushStep::Enqueue(events) => {
                for event in *events {
                    outcome.depths.push(queue.enqueue((*event).to_owned()));
                }
            }
            FlushStep::Flush { reply, arrivals } => {
                let (report, sent, delivered) = flush_once(&queue, *reply, arrivals).await;
                outcome.reports.push(report);
                outcome.attempts.push(sent);
                outcome.delivered.extend(delivered);
            }
        }
    }
    outcome.queue = queue.pending();
    outcome
}

fn owned(events: &[&str]) -> Vec<String> {
    events.iter().map(|event| (*event).to_owned()).collect()
}

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
        depths: &[],
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
        depths: &[1, 2],
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
        depths: &[1, 2],
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
            FlushStep::Enqueue(&["a", "b", "c", "d"]),
            FlushStep::Flush { reply: Reply::Failed, arrivals: &[] },
        ],
    },
    Expectation {
        sent: &[&["a", "b", "c", "d"]],
        delivered: &[],
        queue: &["b", "c", "d"],
        depths: &[1, 2, 3, 4],
        reports: &[ReportExpectation { batch: 4, delivered: 0, requeued: 4, dropped_oldest: 1, cancelled: false }],
        warns: true,
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
        depths: &[1, 2],
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
        depths: &[1, 2, 3],
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
        depths: &[1, 2],
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
        depths: &[1, 2],
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
        depths: &[1, 2, 3],
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
    assert_eq!(outcome.depths, expected.depths);
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
