#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use betterlite_sdk::call_wrapper::ports::{IdSource, WrapperClock};
use chrono::{DateTime, FixedOffset};
use uuid::Uuid;

use super::super::source::SilentSink;
use super::{SessionCore, SystemClock, SystemIds};

const SESSION_TEXT: &str =
    r#"{"environment":{},"api_key":"sk-test","api_base":"http://127.0.0.1:9/v1"}"#;
const FIXED_SECONDS: i64 = 1_700_000_000;
const MILLIS_PER_SECOND: f64 = 1000.0;
const WAIT: Duration = Duration::from_millis(5);

struct StepClock {
    next: AtomicU64,
}

impl StepClock {
    fn instant() -> DateTime<FixedOffset> {
        DateTime::from_timestamp(FIXED_SECONDS, 0)
            .expect("the fixed instant is in range")
            .fixed_offset()
    }
}

impl WrapperClock for StepClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Self::instant()
    }

    // each reading is a second after the last, so the message copy LoggingState times is one second
    fn monotonic(&self) -> Duration {
        let step = self.next.fetch_add(1, Ordering::Relaxed);
        Duration::from_secs(step)
    }
}

struct StepIds {
    next: AtomicUsize,
}

impl IdSource for StepIds {
    fn next_call_id(&self) -> String {
        let step = self.next.fetch_add(1, Ordering::Relaxed);
        format!("id-{step}")
    }
}

#[test]
fn a_session_reads_the_injected_clock_and_ids() {
    let session = SessionCore::build(
        SESSION_TEXT,
        Arc::new(StepClock {
            next: AtomicU64::new(0),
        }),
        Arc::new(StepIds {
            next: AtomicUsize::new(1),
        }),
    )
    .expect("the session builds");
    let state = session
        .logging_state("vector_store_create")
        .expect("the logging state builds");
    assert_eq!(state.start_time, StepClock::instant());
    assert_eq!(state.call_type, "vector_store_create");
    assert_eq!(state.litellm_call_id, "id-1");
    assert_eq!(state.litellm_trace_id, "id-2");
    assert!((state.message_copy_duration_ms - MILLIS_PER_SECOND).abs() < f64::EPSILON);
    let sink = SilentSink;
    assert_eq!(session.pre_post(&sink).now, StepClock::instant());
}

#[test]
fn system_ids_give_distinct_uuids() {
    let ids = SystemIds;
    let first = ids.next_call_id();
    let second = ids.next_call_id();
    assert_ne!(first, second);
    Uuid::parse_str(&first).expect("a call id is a uuid4");
}

#[test]
fn system_clock_reports_utc_time_that_advances() {
    let clock = SystemClock::new();
    let start = clock.monotonic();
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {}
    assert!(clock.monotonic() > start);
    assert!(clock.now().timestamp() > 0);
    assert_eq!(clock.now().offset().local_minus_utc(), 0);
}
