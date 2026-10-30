#![cfg(test)]

use super::reconnect::{DbReconnect, RetryPolicy, call_with_db_reconnect_retry};
use super::*;
use std::error::Error as _;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

fn error(kind: ErrorType) -> DbError {
    DbError {
        error_type: kind,
        message: String::new(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

#[test]
fn connection_allowlist() {
    assert!(is_database_connection_error(&error(ErrorType::Deadline)));
    assert!(!is_database_connection_error(&error(ErrorType::Prisma)));
}
#[test]
fn infrastructure_data_exclusions() {
    assert!(!is_database_infrastructure(&error(
        ErrorType::UniqueViolation
    )));
    assert!(is_database_infrastructure(&error(ErrorType::Prisma)));
}
#[test]
fn base_data_exact_type() {
    assert!(is_prisma_data_error(&error(ErrorType::Data)));
    assert!(!is_prisma_data_error(&error(ErrorType::RawQuery)));
}
#[test]
fn p1001_base_data() {
    let mut value = error(ErrorType::Data);
    value.message = "can't reach database server".to_owned();
    assert!(is_prisma_data_error(&value));
}

#[test]
fn unique_violation_uses_meta_sqlstate() {
    let mut value = error(ErrorType::UniqueViolation);
    value.meta_code = Some("23505".to_owned());
    assert_eq!(postgres_sqlstate(&value).as_deref(), Some("23505"));
}

#[rstest::rstest]
#[case(ErrorType::EngineConnection)]
#[case(ErrorType::ClientNotConnected)]
#[case(ErrorType::HttpClientClosed)]
fn prisma_subclasses_cover_all_predicates(#[case] error_type: ErrorType) {
    let value = error(error_type);
    assert!(is_prisma_error(&value));
    assert!(is_database_infrastructure(&value));
    assert!(!is_database_infrastructure(&error(ErrorType::Deadline)));
}

#[test]
fn deadline_and_engine_transport_are_distinct() {
    assert!(!is_database_transport_error(&error(ErrorType::Deadline)));
    let mut engine = error(ErrorType::EngineConnection);
    assert!(!is_database_transport_error(&engine));
    engine.message = "connection refused".to_owned();
    assert!(is_database_transport_error(&engine));
}
#[test]
fn transport_keyword_matrix() {
    let mut value = error(ErrorType::Prisma);
    value.message = "NETWORK IS UNREACHABLE".to_owned();
    assert!(is_database_transport_error(&value));
    assert!(is_database_transport_error(&error(
        ErrorType::HttpClientClosed
    )));
}
#[test]
fn unrelated_message_not_transport() {
    let mut value = error(ErrorType::Prisma);
    value.message = "query rejected".to_owned();
    assert!(!is_database_transport_error(&value));
}
#[test]
fn deadlock_code_and_phrases() {
    let mut value = error(ErrorType::Prisma);
    value.code = Some("P2034".to_owned());
    assert!(is_deadlock_error(&value));
}
#[test]
fn read_only_code_and_phrase() {
    let mut value = error(ErrorType::Prisma);
    value.message = "ERROR \"25006\"".to_owned();
    assert!(is_read_only_transaction_error(&value));
}
#[test]
fn meta_code_precedes_batch() {
    let mut value = error(ErrorType::Data);
    value.meta_code = Some("23505".to_owned());
    value.message = "PostgresError { code: \"40001\"".to_owned();
    assert_eq!(postgres_sqlstate(&value).as_deref(), Some("23505"));
}
#[test]
fn malformed_meta_batch_fallback() {
    let mut value = error(ErrorType::Data);
    value.message = "PostgresError { code: \"40001\"".to_owned();
    assert_eq!(postgres_sqlstate(&value).as_deref(), Some("40001"));
}
#[test]
fn batch_uppercase_five_chars() {
    let mut value = error(ErrorType::Data);
    value.message = "PostgresError { code: \"ABCDE\"".to_owned();
    assert_eq!(postgres_sqlstate(&value).as_deref(), Some("ABCDE"));
}
#[test]
fn nondata_no_sqlstate() {
    assert_eq!(postgres_sqlstate(&error(ErrorType::Prisma)), None);
}
#[test]
fn engine_traceback_origin() {
    let mut value = error(ErrorType::Other);
    value
        .traceback_modules
        .push("prisma.engine.response".to_owned());
    assert!(is_prisma_engine_internal(&value));
}
#[test]
fn known_prisma_not_engine_internal() {
    let mut value = error(ErrorType::Prisma);
    value
        .traceback_modules
        .push("prisma.engine.response".to_owned());
    assert!(!is_prisma_engine_internal(&value));
}
#[test]
fn permanent_excludes_transient() {
    assert!(is_permanent_database_fault(&error(ErrorType::Prisma)));
    assert!(!is_permanent_database_fault(&error(ErrorType::Connection)));
}
#[test]
fn exception_classes_only() {
    assert!(!is_prisma_error(&error(ErrorType::Other)));
}

#[tokio::test]
async fn read_success() {
    let mut client = ScriptedClient {
        effects: Arc::new(Mutex::new(RetryEffects::default())),
        reconnect: Ok(true),
        timeout: None,
        lock_timeout: None,
    };
    let result = call_with_db_reconnect_retry(
        &mut client,
        || async { Ok::<_, DbError>(7_i32) },
        RetryPolicy {
            reason: "read",
            retry_safe_error_types: None,
            timeout_seconds: None,
            lock_timeout_seconds: None,
        },
    )
    .await;
    assert_eq!(result.ok(), Some(7_i32));
}

#[derive(Default)]
struct RetryEffects {
    reconnects: Vec<(String, f64, f64)>,
}

struct ScriptedClient {
    effects: Arc<Mutex<RetryEffects>>,
    reconnect: Result<bool, DbError>,
    timeout: Option<f64>,
    lock_timeout: Option<f64>,
}

impl DbReconnect for ScriptedClient {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        self.timeout
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        self.lock_timeout
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        self.effects.lock().expect("test mutex").reconnects.push((
            reason.to_owned(),
            timeout_seconds,
            lock_timeout_seconds,
        ));
        let reconnect = self.reconnect.clone();
        Box::pin(async move { reconnect })
    }
}

fn transport_error() -> DbError {
    let mut value = error(ErrorType::Connection);
    value.message = "connection refused".to_owned();
    value
}

async fn scripted_retry(
    reads: Vec<Result<i32, DbError>>,
    reconnect: Result<bool, DbError>,
    timeout: Option<f64>,
    lock_timeout: Option<f64>,
) -> (Result<i32, DbError>, RetryEffects) {
    let effects = Arc::new(Mutex::new(RetryEffects::default()));
    let mut client = ScriptedClient {
        effects: Arc::clone(&effects),
        reconnect,
        timeout,
        lock_timeout,
    };
    let pending_reads = Arc::new(Mutex::new(reads));
    let result = call_with_db_reconnect_retry(
        &mut client,
        || {
            let pending = Arc::clone(&pending_reads);
            async move { pending.lock().expect("test mutex").remove(0) }
        },
        RetryPolicy {
            reason: "read",
            retry_safe_error_types: None,
            timeout_seconds: None,
            lock_timeout_seconds: None,
        },
    )
    .await;
    let recorded_effects = mem::take(&mut *effects.lock().expect("test mutex"));
    (result, recorded_effects)
}

#[tokio::test]
async fn data_no_reconnect() {
    let (result, effects) =
        scripted_retry(vec![Err(error(ErrorType::Data))], Ok(true), None, None).await;
    drop(result.expect_err("data error does not reconnect"));
    assert!(effects.reconnects.is_empty());
}

#[tokio::test]
async fn unsafe_transport_no_replay() {
    let (result, effects) =
        scripted_retry(vec![Err(error(ErrorType::Deadline))], Ok(true), None, None).await;
    drop(result.expect_err("deadline does not reconnect"));
    assert!(effects.reconnects.is_empty());
}

#[tokio::test]
async fn absent_reconnect() {
    let (result, effects) =
        scripted_retry(vec![Err(transport_error())], Ok(false), None, None).await;
    drop(result.expect_err("false reconnect preserves first"));
    assert_eq!(effects.reconnects.len(), 1);
}

#[tokio::test]
async fn timeout_argument_attribute_defaults() {
    let (result, effects) = scripted_retry(
        vec![Err(transport_error()), Ok(4_i32)],
        Ok(true),
        Some(3.0_f64),
        Some(0.25_f64),
    )
    .await;
    assert_eq!(result.ok(), Some(4_i32));
    assert_eq!(
        effects.reconnects[0],
        ("read".to_owned(), 3.0_f64, 0.25_f64)
    );
}

#[tokio::test]
async fn timeout_bool_string_fallback() {
    let (result, effects) = scripted_retry(
        vec![Err(transport_error()), Ok(6_i32)],
        Ok(true),
        None,
        None,
    )
    .await;
    assert_eq!(result.ok(), Some(6_i32));
    assert_eq!(effects.reconnects[0].1.to_bits(), 2.0_f64.to_bits());
    assert_eq!(effects.reconnects[0].2.to_bits(), 0.1_f64.to_bits());
}

#[tokio::test]
async fn reconnect_warning_arguments() {
    let (result, effects) = scripted_retry(
        vec![Err(transport_error()), Ok(7_i32)],
        Ok(true),
        None,
        None,
    )
    .await;
    assert_eq!(result.ok(), Some(7_i32));
    assert_eq!(effects.reconnects[0].0, "read");
}

#[tokio::test]
async fn retry_safe_types_prevent_replay() {
    let mut client = ScriptedClient {
        effects: Arc::new(Mutex::new(RetryEffects::default())),
        reconnect: Ok(true),
        timeout: None,
        lock_timeout: None,
    };
    let mut engine = transport_error();
    engine.error_type = ErrorType::EngineConnection;
    let allowed = [ErrorType::Connection];
    let result = call_with_db_reconnect_retry(
        &mut client,
        || async { Err::<i32, _>(engine.clone()) },
        RetryPolicy {
            reason: "write",
            retry_safe_error_types: Some(&allowed),
            timeout_seconds: None,
            lock_timeout_seconds: None,
        },
    )
    .await;
    drop(result.expect_err("unsafe retry type is rejected"));
    assert!(
        client
            .effects
            .lock()
            .expect("test mutex")
            .reconnects
            .is_empty()
    );
}

#[tokio::test]
async fn numeric_timeout_passthrough() {
    let (result, effects) = scripted_retry(
        vec![Err(transport_error()), Ok(5_i32)],
        Ok(true),
        Some(8.0_f64),
        Some(0.5_f64),
    )
    .await;
    assert_eq!(result.ok(), Some(5_i32));
    assert_eq!(effects.reconnects[0].1.to_bits(), 8.0_f64.to_bits());
}

#[tokio::test]
async fn reconnect_false_preserves_first() {
    let first = transport_error();
    let (result, _) = scripted_retry(vec![Err(first.clone())], Ok(false), None, None).await;
    assert_eq!(
        result.as_ref().err().map(|value| value.message.as_str()),
        Some(first.message.as_str())
    );
}

#[tokio::test]
async fn reconnect_raise_chains_first() {
    let first = transport_error();
    let reconnect_error = error(ErrorType::Other);
    let (result, _) =
        scripted_retry(vec![Err(first.clone())], Err(reconnect_error), None, None).await;
    let value = result.expect_err("reconnect raises");
    assert_eq!(value.message, first.message);
    assert_eq!(value.source().map(ToString::to_string), Some(String::new()));
}

#[tokio::test]
async fn retry_success() {
    let (result, effects) = scripted_retry(
        vec![Err(transport_error()), Ok(9_i32)],
        Ok(true),
        None,
        None,
    )
    .await;
    assert_eq!(result.ok(), Some(9_i32));
    assert_eq!(effects.reconnects.len(), 1);
}

#[tokio::test]
async fn retry_failure_no_loop() {
    let (result, effects) = scripted_retry(
        vec![Err(transport_error()), Err(transport_error())],
        Ok(true),
        None,
        None,
    )
    .await;
    drop(result.expect_err("retry failure does not loop"));
    assert_eq!(effects.reconnects.len(), 1);
}
