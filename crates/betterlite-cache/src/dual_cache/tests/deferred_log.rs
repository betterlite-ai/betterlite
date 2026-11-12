use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_redis::batch::error::BatchError;
use tracing::Subscriber;
use tracing::subscriber::with_default;

use crate::dual_cache::increments::log_deferred_increment_failure;

struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
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

pub(crate) fn capture_warnings() -> (impl Subscriber + Send + Sync, Arc<Mutex<Vec<u8>>>) {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&bytes);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_ansi(false)
        .with_writer(move || Capture(Arc::clone(&sink)))
        .finish();
    (subscriber, bytes)
}

pub(crate) fn captured(bytes: &Mutex<Vec<u8>>) -> String {
    let log = bytes.lock().unwrap_or_else(PoisonError::into_inner).clone();
    String::from_utf8(log).expect("the log is utf-8")
}

fn logged_for(error: BatchError) -> String {
    let (subscriber, bytes) = capture_warnings();
    let follow_up = with_default(subscriber, || {
        log_deferred_increment_failure(&Err(Arc::new(error)))
    });
    assert!(follow_up.is_none());
    captured(&bytes)
}

#[test]
fn a_failed_post_call_increment_is_logged() {
    let log = logged_for(BatchError::MissingReply {
        operation: "increment",
    });
    assert!(log.contains("post-call Redis increment failed"), "{log}");
}

#[test]
fn a_cancelled_post_call_increment_is_not_logged() {
    assert_eq!(logged_for(BatchError::Cancelled), "");
}
