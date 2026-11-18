#![cfg(test)]

use super::{DB_RETRY_SAFE_ERROR_TYPES, RetryMode};
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::database_faults::{DbError, ErrorType};
use std::future::{Future, Ready, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Clone, Default)]
struct Effects(Arc<Mutex<Vec<String>>>);

impl Effects {
    fn push(&self, effect: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect.to_owned());
    }

    fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct FakeClient {
    effects: Effects,
    reconnect: Result<bool, DbError>,
}

impl DbReconnect for FakeClient {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        Some(5.0)
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        Some(0.25)
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        self.effects.push(&format!(
            "reconnect {reason} {timeout_seconds} {lock_timeout_seconds}"
        ));
        let result = self.reconnect.clone();
        Box::pin(async move { result })
    }
}

fn error(kind: ErrorType, message: &str) -> DbError {
    DbError {
        error_type: kind,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

fn operation(
    effects: Effects,
    outcomes: Vec<Result<i32, DbError>>,
) -> impl FnMut() -> Ready<Result<i32, DbError>> {
    let mut remaining = outcomes.into_iter();
    move || {
        effects.push("run");
        ready(
            remaining
                .next()
                .expect("the retry helper ran more attempts than scripted"),
        )
    }
}

fn client(effects: &Effects, reconnect: Result<bool, DbError>) -> FakeClient {
    FakeClient {
        effects: effects.clone(),
        reconnect,
    }
}

#[tokio::test]
async fn retry_read_reconnects_once_and_replays_a_connect_error() {
    let effects = Effects::default();
    let mut fake = client(&effects, Ok(true));
    let call = operation(
        effects.clone(),
        vec![
            Err(error(ErrorType::Connect, "connection refused")),
            Ok(7_i32),
        ],
    );
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Read.policy("reset_budget_read_keys_failure"),
    )
    .await;
    assert_eq!(result, Ok(7_i32));
    assert_eq!(
        effects.entries(),
        [
            "run",
            "reconnect reset_budget_read_keys_failure 5 0.25",
            "run",
        ]
    );
}

#[tokio::test]
async fn retry_read_replays_an_ambiguous_read_error() {
    let effects = Effects::default();
    let mut fake = client(&effects, Ok(true));
    let call = operation(
        effects.clone(),
        vec![Err(error(ErrorType::ReadTimeout, "timed out")), Ok(7_i32)],
    );
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Read.policy("reset_budget_read_keys_failure"),
    )
    .await;
    assert_eq!(result, Ok(7_i32));
    assert_eq!(effects.entries().len(), 3);
}

#[tokio::test]
async fn retry_read_replays_a_lost_connection() {
    let effects = Effects::default();
    let mut fake = client(&effects, Ok(true));
    let call = operation(
        effects.clone(),
        vec![
            Err(error(ErrorType::ClientNotConnected, "not connected")),
            Ok(7_i32),
        ],
    );
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Read.policy("reset_budget_read_keys_failure"),
    )
    .await;
    assert_eq!(result, Ok(7_i32));
    assert_eq!(effects.entries().len(), 3);
}

#[tokio::test]
async fn retry_write_replays_a_connect_error() {
    let effects = Effects::default();
    let mut fake = client(&effects, Ok(true));
    let call = operation(
        effects.clone(),
        vec![
            Err(error(ErrorType::Connect, "connection refused")),
            Ok(7_i32),
        ],
    );
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Write.policy("reset_budget_write_keys_failure"),
    )
    .await;
    assert_eq!(result, Ok(7_i32));
    assert_eq!(
        effects.entries(),
        [
            "run",
            "reconnect reset_budget_write_keys_failure 5 0.25",
            "run",
        ]
    );
}

#[tokio::test]
async fn retry_write_does_not_replay_an_ambiguous_read_error() {
    let effects = Effects::default();
    let mut fake = client(&effects, Ok(true));
    let first = error(ErrorType::ReadTimeout, "timed out");
    let call = operation(effects.clone(), vec![Err(first.clone()), Ok(7_i32)]);
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Write.policy("reset_budget_write_keys_failure"),
    )
    .await;
    assert_eq!(result, Err(first));
    assert_eq!(effects.entries(), ["run"]);
}

#[tokio::test]
async fn retry_write_does_not_replay_a_lost_connection() {
    let effects = Effects::default();
    let mut fake = client(&effects, Ok(true));
    let first = error(ErrorType::ClientNotConnected, "not connected");
    let call = operation(effects.clone(), vec![Err(first.clone()), Ok(7_i32)]);
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Write.policy("reset_budget_write_keys_failure"),
    )
    .await;
    assert_eq!(result, Err(first));
    assert_eq!(effects.entries(), ["run"]);
}

#[tokio::test]
async fn retry_non_transport_error_is_not_replayed() {
    let effects = Effects::default();
    let mut fake = client(&effects, Ok(true));
    let first = error(ErrorType::Data, "statement refused");
    let call = operation(effects.clone(), vec![Err(first.clone()), Ok(7_i32)]);
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Read.policy("reset_budget_read_keys_failure"),
    )
    .await;
    assert_eq!(result, Err(first));
    assert_eq!(effects.entries(), ["run"]);
}

#[tokio::test]
async fn retry_stops_after_one_replay() {
    let effects = Effects::default();
    let mut fake = client(&effects, Ok(true));
    let second = error(ErrorType::Connect, "second failure");
    let call = operation(
        effects.clone(),
        vec![
            Err(error(ErrorType::Connect, "first failure")),
            Err(second.clone()),
            Ok(7_i32),
        ],
    );
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Read.policy("reset_budget_read_keys_failure"),
    )
    .await;
    assert_eq!(result, Err(second));
    assert_eq!(effects.entries().len(), 3);
}

#[tokio::test]
async fn retry_keeps_the_transport_error_when_the_reconnect_fails() {
    let effects = Effects::default();
    let reconnect_error = error(ErrorType::Connect, "reconnect failed");
    let mut fake = client(&effects, Err(reconnect_error.clone()));
    let first = error(ErrorType::Connect, "connection refused");
    let call = operation(effects.clone(), vec![Err(first.clone())]);
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Read.policy("reset_budget_read_keys_failure"),
    )
    .await;
    let expected = DbError {
        cause: Some(Box::new(reconnect_error)),
        ..first
    };
    assert_eq!(result, Err(expected));
    assert_eq!(effects.entries().len(), 2);
}

#[tokio::test]
async fn retry_returns_the_transport_error_when_the_reconnect_is_refused() {
    let effects = Effects::default();
    let mut fake = client(&effects, Ok(false));
    let first = error(ErrorType::Connect, "connection refused");
    let call = operation(effects.clone(), vec![Err(first.clone()), Ok(7_i32)]);
    let result = call_with_db_reconnect_retry(
        Some(&mut fake),
        call,
        RetryMode::Read.policy("reset_budget_read_keys_failure"),
    )
    .await;
    assert_eq!(result, Err(first));
    assert_eq!(effects.entries().len(), 2);
}

#[tokio::test]
async fn retry_without_a_client_returns_the_transport_error() {
    let effects = Effects::default();
    let first = error(ErrorType::Connect, "connection refused");
    let call = operation(effects.clone(), vec![Err(first.clone()), Ok(7_i32)]);
    let missing: Option<&mut FakeClient> = None;
    let result = call_with_db_reconnect_retry(
        missing,
        call,
        RetryMode::Read.policy("reset_budget_read_keys_failure"),
    )
    .await;
    assert_eq!(result, Err(first));
    assert_eq!(effects.entries(), ["run"]);
}

#[test]
fn retry_write_policy_narrows_replays_to_the_connect_class() {
    let read = RetryMode::Read.policy("reset_budget_read_keys_failure");
    assert_eq!(read.reason, "reset_budget_read_keys_failure");
    assert_eq!(read.retry_safe_error_types, None);
    assert_eq!(read.timeout_seconds, None);
    assert_eq!(read.lock_timeout_seconds, None);
    let write = RetryMode::Write.policy("reset_budget_write_keys_failure");
    assert_eq!(write.reason, "reset_budget_write_keys_failure");
    assert_eq!(
        write.retry_safe_error_types,
        Some(DB_RETRY_SAFE_ERROR_TYPES)
    );
    assert_eq!(DB_RETRY_SAFE_ERROR_TYPES.len(), 1);
    assert!(DB_RETRY_SAFE_ERROR_TYPES.contains(&ErrorType::Connect));
}
