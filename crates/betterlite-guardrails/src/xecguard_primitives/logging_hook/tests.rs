#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use tracing::subscriber::with_default;

use super::logging_hook;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_debug() -> (Captured, impl tracing::Subscriber + Send + Sync) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, subscriber)
}

#[test]
fn logging_hook_runs_the_async_hook_when_no_loop_is_running() {
    let (kwargs, result) = logging_hook(
        vec!["request".to_owned()],
        1_i32,
        async |kwargs: &mut Vec<String>, result: &mut i32| {
            kwargs.push("scanned".to_owned());
            *result += 1_i32;
            Ok::<(), String>(())
        },
    );
    assert_eq!(kwargs, vec!["request".to_owned(), "scanned".to_owned()]);
    assert_eq!(result, 2_i32);
}

#[tokio::test]
async fn logging_hook_returns_the_pair_untouched_on_a_running_loop() {
    let (kwargs, result) = logging_hook(
        vec!["request".to_owned()],
        1_i32,
        async |kwargs: &mut Vec<String>, result: &mut i32| {
            kwargs.push("scanned".to_owned());
            *result += 1_i32;
            Ok::<(), String>(())
        },
    );
    assert_eq!(kwargs, vec!["request".to_owned()]);
    assert_eq!(result, 1_i32);
}

#[test]
fn logging_hook_swallows_an_async_error_and_returns_the_pair() {
    let (captured, subscriber) = capture_debug();
    let (kwargs, result) = with_default(subscriber, || {
        logging_hook(
            vec!["request".to_owned()],
            1_i32,
            async |_: &mut Vec<String>, _: &mut i32| Err::<(), String>("boom".to_owned()),
        )
    });
    assert_eq!(kwargs, vec!["request".to_owned()]);
    assert_eq!(result, 1_i32);
    let text =
        String::from_utf8(captured.0.lock().expect("captured log").clone()).expect("utf8 log");
    assert!(text.contains("swallowed exception"));
    assert!(text.contains("boom"));
}

#[test]
fn logging_hook_keeps_mutations_an_async_hook_made_before_failing() {
    let (captured, subscriber) = capture_debug();
    let (kwargs, _result) = with_default(subscriber, || {
        logging_hook(
            vec!["request".to_owned()],
            1_i32,
            async |kwargs: &mut Vec<String>, _: &mut i32| {
                kwargs.push("observed".to_owned());
                Err::<(), String>("late failure".to_owned())
            },
        )
    });
    assert_eq!(kwargs, vec!["request".to_owned(), "observed".to_owned()]);
    let text =
        String::from_utf8(captured.0.lock().expect("captured log").clone()).expect("utf8 log");
    assert!(text.contains("late failure"));
}
