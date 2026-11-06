#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use super::{
    BatchCancelled, BatchResponse, BatchTransport, DeliverySettings, SendFailure,
    send_batch_with_413_split,
};
use crate::batch::retry::RequeuePolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Response { status: u16, detail: &'static str },
    StatusError { status: u16, detail: &'static str },
    Failed,
    Cancel,
}

const OK: Step = Step::Response {
    status: 200,
    detail: "the scripted provider answered",
};
const ACCEPTED: Step = Step::Response {
    status: 202,
    detail: "the scripted provider accepted the batch",
};
const BAD_REQUEST: Step = Step::Response {
    status: 400,
    detail: "the scripted provider rejected the request",
};
const TOO_LARGE: Step = Step::Response {
    status: 413,
    detail: "the scripted provider rejected the payload",
};
const SERVER_ERROR: Step = Step::Response {
    status: 500,
    detail: "the scripted provider failed",
};
const RAISED_TOO_LARGE: Step = Step::StatusError {
    status: 413,
    detail: "the scripted provider rejected the payload",
};
const RAISED_TOO_MANY: Step = Step::StatusError {
    status: 429,
    detail: "the scripted provider asked for a slower retry",
};
const PROVIDER_DETAIL: Step = Step::Response {
    status: 400,
    detail: "quota exhausted for this key",
};
const UNREACHABLE: Step = Step::Failed;
const CANCELLED: Step = Step::Cancel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Policy {
    Undelivered,
    Requeue,
}

#[derive(Debug, Clone, Copy)]
struct SendCase {
    events: &'static [&'static str],
    success_status_codes: &'static [u16],
    policy: Policy,
    max_events_per_request: Option<usize>,
    oversized: &'static [&'static str],
    unserializable: &'static [&'static str],
    script: &'static [Step],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Expectation {
    attempts: &'static [&'static [&'static str]],
    delivered: &'static [&'static str],
    undelivered: &'static [&'static str],
    cancelled: bool,
    logs: &'static [&'static str],
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

#[derive(Debug, thiserror::Error)]
#[error("scripted transport failure")]
struct ScriptedFailure;

struct Attempt {
    events: Vec<String>,
}

struct ScriptedTransport {
    script: Mutex<VecDeque<Step>>,
    attempts: Mutex<Vec<Attempt>>,
    delivered: Mutex<Vec<String>>,
    success_status_codes: Vec<u16>,
    max_events_per_request: Option<usize>,
    oversized: Vec<String>,
    unserializable: Vec<String>,
}

impl BatchTransport<String> for ScriptedTransport {
    type SendError = ScriptedFailure;
    type LimitError = ScriptedFailure;

    fn send(
        &self,
        batch: &[String],
    ) -> impl Future<Output = Result<BatchResponse, SendFailure<Self::SendError>>> + Send {
        let step = self.script.lock().unwrap().pop_front().unwrap();
        let events = batch.to_vec();
        let success = matches!(
            step,
            Step::Response { status, .. } if self.success_status_codes.contains(&status)
        );
        if success {
            self.delivered
                .lock()
                .unwrap()
                .extend(events.iter().cloned());
        }
        self.attempts.lock().unwrap().push(Attempt { events });
        async move {
            match step {
                Step::Response { status, detail } => Ok(BatchResponse {
                    status,
                    detail: detail.to_owned(),
                }),
                Step::StatusError { status, detail } => Err(SendFailure::Status {
                    status,
                    detail: detail.to_owned(),
                }),
                UNREACHABLE => Err(SendFailure::Other(ScriptedFailure)),
                CANCELLED => Err(SendFailure::Cancelled),
            }
        }
    }

    fn exceeds_limits(&self, batch: &[String]) -> Result<bool, Self::LimitError> {
        if batch
            .iter()
            .any(|event| self.unserializable.contains(event))
        {
            return Err(ScriptedFailure);
        }
        if batch.iter().any(|event| self.oversized.contains(event)) {
            return Ok(true);
        }
        Ok(self
            .max_events_per_request
            .is_some_and(|max| batch.len() > max))
    }
}

async fn drive(
    case: &SendCase,
) -> (
    Result<Vec<String>, BatchCancelled<String>>,
    ScriptedTransport,
) {
    let transport = ScriptedTransport {
        script: Mutex::new(case.script.iter().copied().collect()),
        attempts: Mutex::new(Vec::new()),
        delivered: Mutex::new(Vec::new()),
        success_status_codes: case.success_status_codes.to_vec(),
        max_events_per_request: case.max_events_per_request,
        oversized: owned(case.oversized),
        unserializable: owned(case.unserializable),
    };
    let events = owned(case.events);
    let policy = match case.policy {
        Policy::Undelivered => RequeuePolicy::Undelivered,
        Policy::Requeue => RequeuePolicy::RequeueAll,
    };
    let settings = DeliverySettings {
        success_status_codes: case.success_status_codes,
        policy,
        integration_name: "BatchDelivery",
        drop_error_message: "batch delivery dropped a single event the destination rejected as too large",
    };
    let settled = AtomicUsize::new(0);
    let result = send_batch_with_413_split(&transport, &events, settings, &settled).await;
    (result, transport)
}

fn owned(events: &[&str]) -> Vec<String> {
    events.iter().map(|event| (*event).to_owned()).collect()
}

#[rstest]
#[case::response_413_splits_the_batch(
    SendCase {
        events: &["e0", "e1", "e2", "e3"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[TOO_LARGE, OK, SERVER_ERROR],
    },
    Expectation {
        attempts: &[&["e0", "e1", "e2", "e3"], &["e0", "e1"], &["e2", "e3"]],
        delivered: &["e0", "e1"],
        undelivered: &["e2", "e3"],
        cancelled: false,
        logs: &["a retry can clear"],
    }
)]
#[case::raised_413_splits_the_batch(
    SendCase {
        events: &["e0", "e1"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[RAISED_TOO_LARGE, OK, OK],
    },
    Expectation {
        attempts: &[&["e0", "e1"], &["e0"], &["e1"]],
        delivered: &["e0", "e1"],
        undelivered: &[],
        cancelled: false,
        logs: &[],
    }
)]
#[case::single_event_413_is_dropped(
    SendCase {
        events: &["e0"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[TOO_LARGE],
    },
    Expectation {
        attempts: &[&["e0"]],
        delivered: &[],
        undelivered: &[],
        cancelled: false,
        logs: &["rejected as too large"],
    }
)]
#[case::oversized_single_event_is_sent_then_dropped(
    SendCase {
        events: &["e0"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &["e0"],
        unserializable: &[],
        script: &[TOO_LARGE],
    },
    Expectation {
        attempts: &[&["e0"]],
        delivered: &[],
        undelivered: &[],
        cancelled: false,
        logs: &["rejected as too large"],
    }
)]
#[case::oversized_batch_splits_before_sending(
    SendCase {
        events: &["e0", "e1", "e2", "e3"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &["e2"],
        unserializable: &[],
        script: &[OK, OK, OK],
    },
    Expectation {
        attempts: &[&["e0", "e1"], &["e2"], &["e3"]],
        delivered: &["e0", "e1", "e2", "e3"],
        undelivered: &[],
        cancelled: false,
        logs: &[],
    }
)]
#[case::count_limit_splits_before_sending(
    SendCase {
        events: &["e0", "e1", "e2"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: Some(2),
        oversized: &[],
        unserializable: &[],
        script: &[OK, OK],
    },
    Expectation {
        attempts: &[&["e0"], &["e1", "e2"]],
        delivered: &["e0", "e1", "e2"],
        undelivered: &[],
        cancelled: false,
        logs: &[],
    }
)]
#[case::unserializable_batch_splits_until_the_record_is_isolated(
    SendCase {
        events: &["e0", "e1", "e2", "e3"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &["e2"],
        script: &[OK, OK],
    },
    Expectation {
        attempts: &[&["e0", "e1"], &["e3"]],
        delivered: &["e0", "e1", "e3"],
        undelivered: &[],
        cancelled: false,
        logs: &["cannot be serialized"],
    }
)]
#[case::unserializable_single_record_is_dropped(
    SendCase {
        events: &["e0"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &["e0"],
        script: &[],
    },
    Expectation {
        attempts: &[],
        delivered: &[],
        undelivered: &[],
        cancelled: false,
        logs: &["cannot be serialized"],
    }
)]
#[case::failed_left_half_skips_the_right_half(
    SendCase {
        events: &["e0", "e1", "e2", "e3"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[TOO_LARGE, SERVER_ERROR],
    },
    Expectation {
        attempts: &[&["e0", "e1", "e2", "e3"], &["e0", "e1"]],
        delivered: &[],
        undelivered: &["e0", "e1", "e2", "e3"],
        cancelled: false,
        logs: &["a retry can clear"],
    }
)]
#[case::non_retryable_status_drops_the_batch(
    SendCase {
        events: &["e0", "e1"],
        success_status_codes: &[200],
        policy: Policy::Undelivered,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[BAD_REQUEST],
    },
    Expectation {
        attempts: &[&["e0", "e1"]],
        delivered: &[],
        undelivered: &[],
        cancelled: false,
        logs: &["a retry cannot clear"],
    }
)]
#[case::provider_detail_reaches_the_log(
    SendCase {
        events: &["e0"],
        success_status_codes: &[200],
        policy: Policy::Undelivered,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[PROVIDER_DETAIL],
    },
    Expectation {
        attempts: &[&["e0"]],
        delivered: &[],
        undelivered: &[],
        cancelled: false,
        logs: &["a retry cannot clear", "quota exhausted for this key"],
    }
)]
#[case::requeue_policy_keeps_a_non_retryable_batch(
    SendCase {
        events: &["e0", "e1"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[BAD_REQUEST],
    },
    Expectation {
        attempts: &[&["e0", "e1"]],
        delivered: &[],
        undelivered: &["e0", "e1"],
        cancelled: false,
        logs: &["a retry can clear"],
    }
)]
#[case::retryable_status_requeues_the_batch(
    SendCase {
        events: &["e0", "e1"],
        success_status_codes: &[200],
        policy: Policy::Undelivered,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[RAISED_TOO_MANY],
    },
    Expectation {
        attempts: &[&["e0", "e1"]],
        delivered: &[],
        undelivered: &["e0", "e1"],
        cancelled: false,
        logs: &["a retry can clear"],
    }
)]
#[case::generic_failure_requeues_the_batch(
    SendCase {
        events: &["e0", "e1"],
        success_status_codes: &[200],
        policy: Policy::Undelivered,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[UNREACHABLE],
    },
    Expectation {
        attempts: &[&["e0", "e1"]],
        delivered: &[],
        undelivered: &["e0", "e1"],
        cancelled: false,
        logs: &["whole batch is requeued", "scripted transport failure"],
    }
)]
#[case::cancellation_keeps_only_the_remainder(
    SendCase {
        events: &["e0", "e1", "e2", "e3"],
        success_status_codes: &[200],
        policy: Policy::Undelivered,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[
            TOO_LARGE,
            TOO_LARGE,
            OK,
            CANCELLED,
        ],
    },
    Expectation {
        attempts: &[&["e0", "e1", "e2", "e3"], &["e0", "e1"], &["e0"], &["e1"]],
        delivered: &["e0"],
        undelivered: &["e1", "e2", "e3"],
        cancelled: true,
        logs: &[],
    }
)]
#[case::empty_batch_sends_nothing(
    SendCase {
        events: &[],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[],
    },
    Expectation {
        attempts: &[],
        delivered: &[],
        undelivered: &[],
        cancelled: false,
        logs: &[],
    }
)]
#[case::accepted_status_codes_are_the_only_success(
    SendCase {
        events: &["e0", "e1"],
        success_status_codes: &[202],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[ACCEPTED],
    },
    Expectation {
        attempts: &[&["e0", "e1"]],
        delivered: &["e0", "e1"],
        undelivered: &[],
        cancelled: false,
        logs: &[],
    }
)]
#[case::odd_batch_halves_at_the_floor(
    SendCase {
        events: &["e0", "e1", "e2", "e3", "e4"],
        success_status_codes: &[200],
        policy: Policy::Requeue,
        max_events_per_request: None,
        oversized: &[],
        unserializable: &[],
        script: &[TOO_LARGE, OK, OK],
    },
    Expectation {
        attempts: &[&["e0", "e1", "e2", "e3", "e4"], &["e0", "e1"], &["e2", "e3", "e4"]],
        delivered: &["e0", "e1", "e2", "e3", "e4"],
        undelivered: &[],
        cancelled: false,
        logs: &[],
    }
)]
#[tokio::test]
async fn batch_delivery_behavior_tests_behavior(
    #[case] case: SendCase,
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
    let (result, transport) = drive(&case).await;
    let attempted: Vec<Vec<String>> = transport
        .attempts
        .lock()
        .unwrap()
        .iter()
        .map(|attempt| attempt.events.clone())
        .collect();
    let expected_attempts: Vec<Vec<String>> = expected
        .attempts
        .iter()
        .map(|events| owned(events))
        .collect();
    assert_eq!(attempted, expected_attempts);
    assert_eq!(
        *transport.delivered.lock().unwrap(),
        owned(expected.delivered)
    );
    match result {
        Ok(undelivered) => {
            assert!(!expected.cancelled);
            assert_eq!(undelivered, owned(expected.undelivered));
        }
        Err(cancelled) => {
            assert!(expected.cancelled);
            assert_eq!(cancelled.undelivered, owned(expected.undelivered));
        }
    }
    let logs = buffer.contents();
    let missing: Vec<&&str> = expected
        .logs
        .iter()
        .filter(|entry| !logs.contains(**entry))
        .collect();
    assert!(missing.is_empty(), "missing {missing:?} in {logs}");
}
