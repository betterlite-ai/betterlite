#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::{RequeuePolicy, is_retryable_status, undelivered_after_http_error};

const EVENTS: [&str; 3] = ["e0", "e1", "e2"];

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

#[rstest]
#[case::informational(100, true)]
#[case::success(200, true)]
#[case::redirection(399, true)]
#[case::bad_request(400, false)]
#[case::request_timeout(408, true)]
#[case::payload_too_large(413, false)]
#[case::too_many_requests(429, true)]
#[case::client_error_tail(499, false)]
#[case::server_error(500, true)]
#[case::server_error_tail(599, true)]
fn batch_delivery_behavior_tests_behavior(#[case] status: u16, #[case] retryable: bool) {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let (undelivered, requeued, retained, logs) = with_default(subscriber, || {
        let undelivered =
            undelivered_after_http_error(&EVENTS, status, "BatchDelivery", "classification");
        let requeued =
            RequeuePolicy::RequeueAll.retained(&EVENTS, status, "BatchDelivery", "classification");
        let retained =
            RequeuePolicy::Undelivered.retained(&EVENTS, status, "BatchDelivery", "classification");
        (undelivered, requeued, retained, buffer.contents())
    });
    assert_eq!(is_retryable_status(status), retryable);
    assert_eq!(retained, undelivered);
    assert_eq!(requeued, EVENTS);
    assert!(logs.contains("BatchDelivery"), "missing the name in {logs}");
    assert!(
        logs.contains("will be retried"),
        "missing the retry log in {logs}"
    );
    if retryable {
        assert_eq!(undelivered, EVENTS);
    } else {
        assert!(undelivered.is_empty());
        assert!(
            logs.contains("are dropped"),
            "missing the drop log in {logs}"
        );
    }
}
