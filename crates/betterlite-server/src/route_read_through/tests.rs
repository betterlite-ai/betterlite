#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;
use rstest::rstest;

use super::{ModelNotFoundCause, route_with_read_through};
use crate::registry_read_through::{RegistryReadThrough, Resync, ResyncError};

const MODEL: &str = "gpt-4o-mini";

#[derive(Clone, Debug, PartialEq)]
enum FakeError {
    Model(ProxyModelNotFoundError),
    Other,
}

impl ModelNotFoundCause for FakeError {
    fn model_not_found(&self) -> Option<&ProxyModelNotFoundError> {
        match self {
            Self::Model(error) => Some(error),
            Self::Other => None,
        }
    }
}

fn not_found(retryable_with_model_read_through: bool) -> Result<&'static str, FakeError> {
    Err(FakeError::Model(ProxyModelNotFoundError::new(
        "acompletion",
        MODEL,
        retryable_with_model_read_through,
    )))
}

fn other_error() -> Result<&'static str, FakeError> {
    Err(FakeError::Other)
}

struct FakeAttempt<E> {
    calls: AtomicUsize,
    results: Mutex<VecDeque<Result<&'static str, E>>>,
}

impl<E> FakeAttempt<E> {
    fn new(results: Vec<Result<&'static str, E>>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            results: Mutex::new(VecDeque::from(results)),
        }
    }

    fn next_result(&self) -> Result<&'static str, E> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.results.lock().unwrap().pop_front().unwrap()
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

struct FakeResync {
    answer: bool,
    keys: Arc<Mutex<Vec<String>>>,
}

impl FakeResync {
    fn new(answer: bool, keys: &Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            answer,
            keys: Arc::clone(keys),
        }
    }
}

impl Resync for FakeResync {
    fn resync(&self, model: &str) -> impl Future<Output = Result<bool, ResyncError>> + Send {
        self.keys.lock().unwrap().push(model.to_owned());
        ready(Ok(self.answer))
    }
}

fn recorded(keys: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    keys.lock().unwrap().clone()
}

#[tokio::test]
async fn retry_runs_the_attempt_twice_and_returns_the_second_result() {
    let attempt = FakeAttempt::new(vec![not_found(true), Ok("second")]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(true, &keys));
    let result = route_with_read_through(Some(MODEL), Some(&read_through), async || {
        attempt.next_result()
    })
    .await;
    assert_eq!(result, Ok("second"));
    assert_eq!(attempt.calls(), 2);
    assert_eq!(recorded(&keys), vec![MODEL.to_owned()]);
}

#[rstest]
#[case::not_found_again(not_found(true))]
#[case::another_error(other_error())]
#[tokio::test]
async fn retry_returns_the_second_attempt_error(#[case] second: Result<&'static str, FakeError>) {
    let attempt = FakeAttempt::new(vec![not_found(true), second.clone(), Ok("third")]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(true, &keys));
    let result = route_with_read_through(Some(MODEL), Some(&read_through), async || {
        attempt.next_result()
    })
    .await;
    assert_eq!(result, second);
    assert_eq!(attempt.calls(), 2);
}

#[rstest]
#[case::plain_model(MODEL)]
#[case::provider_prefixed("openai/gpt-4o")]
#[case::deployment_name("my-deployment")]
#[case::blank_model(" ")]
#[tokio::test]
async fn retry_asks_the_read_through_with_the_model_text(#[case] model: &str) {
    let attempt = FakeAttempt::new(vec![not_found(true), Ok("second")]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(true, &keys));
    let result = route_with_read_through(Some(model), Some(&read_through), async || {
        attempt.next_result()
    })
    .await;
    assert_eq!(result, Ok("second"));
    assert_eq!(attempt.calls(), 2);
    assert_eq!(recorded(&keys), vec![model.to_owned()]);
}

#[tokio::test]
async fn retry_treats_a_proxy_not_found_error_as_the_cause() {
    let attempt = FakeAttempt::new(vec![
        Err(ProxyModelNotFoundError::new("acompletion", MODEL, true)),
        Ok("second"),
    ]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(true, &keys));
    let result = route_with_read_through(Some(MODEL), Some(&read_through), async || {
        attempt.next_result()
    })
    .await;
    assert_eq!(result, Ok("second"));
    assert_eq!(attempt.calls(), 2);
    assert_eq!(recorded(&keys), vec![MODEL.to_owned()]);
}

#[tokio::test]
async fn no_retry_when_the_flag_is_false() {
    let attempt = FakeAttempt::new(vec![not_found(false), Ok("second")]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(true, &keys));
    let result = route_with_read_through(Some(MODEL), Some(&read_through), async || {
        attempt.next_result()
    })
    .await;
    assert_eq!(result, not_found(false));
    assert_eq!(attempt.calls(), 1);
    assert!(recorded(&keys).is_empty());
}

#[tokio::test]
async fn no_retry_without_a_model() {
    let attempt = FakeAttempt::new(vec![not_found(true), Ok("second")]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(true, &keys));
    let result =
        route_with_read_through(None, Some(&read_through), async || attempt.next_result()).await;
    assert_eq!(result, not_found(true));
    assert_eq!(attempt.calls(), 1);
    assert!(recorded(&keys).is_empty());
}

#[tokio::test]
async fn no_retry_with_an_empty_model() {
    let attempt = FakeAttempt::new(vec![not_found(true), Ok("second")]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(true, &keys));
    let result = route_with_read_through(Some(""), Some(&read_through), async || {
        attempt.next_result()
    })
    .await;
    assert_eq!(result, not_found(true));
    assert_eq!(attempt.calls(), 1);
    assert!(recorded(&keys).is_empty());
}

#[tokio::test]
async fn no_retry_without_a_read_through() {
    let attempt = FakeAttempt::new(vec![not_found(true), Ok("second")]);
    let result = route_with_read_through(Some(MODEL), None, async || attempt.next_result()).await;
    assert_eq!(result, not_found(true));
    assert_eq!(attempt.calls(), 1);
}

#[tokio::test]
async fn no_retry_when_the_read_through_misses() {
    let attempt = FakeAttempt::new(vec![not_found(true), Ok("second")]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(false, &keys));
    let result = route_with_read_through(Some(MODEL), Some(&read_through), async || {
        attempt.next_result()
    })
    .await;
    assert_eq!(result, not_found(true));
    assert_eq!(attempt.calls(), 1);
    assert_eq!(recorded(&keys), vec![MODEL.to_owned()]);
}

#[tokio::test]
async fn no_retry_when_the_first_attempt_succeeds() {
    let attempt = FakeAttempt::new(vec![Ok("first"), Ok("second")]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(true, &keys));
    let result = route_with_read_through(Some(MODEL), Some(&read_through), async || {
        attempt.next_result()
    })
    .await;
    assert_eq!(result, Ok("first"));
    assert_eq!(attempt.calls(), 1);
    assert!(recorded(&keys).is_empty());
}

#[tokio::test]
async fn no_retry_when_the_error_is_not_a_model_not_found() {
    let attempt = FakeAttempt::new(vec![Err(FakeError::Other), Ok("second")]);
    let keys = Arc::new(Mutex::new(Vec::new()));
    let read_through = RegistryReadThrough::new(FakeResync::new(true, &keys));
    let result = route_with_read_through(Some(MODEL), Some(&read_through), async || {
        attempt.next_result()
    })
    .await;
    assert_eq!(result, other_error());
    assert_eq!(attempt.calls(), 1);
    assert!(recorded(&keys).is_empty());
}
