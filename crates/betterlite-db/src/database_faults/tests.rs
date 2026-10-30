#![cfg(test)]

use super::*;

fn error(kind: ErrorType) -> DbError {
    DbError::new(kind)
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
    let mut client = TestClient;
    let result = call_with_db_reconnect_retry(
        &mut client,
        || async { Ok::<_, DbError>(7) },
        "read",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(result.ok(), Some(7));
}

struct TestClient;
impl DbReconnect for TestClient {
    fn attempt_db_reconnect<'a>(
        &'a mut self,
        _reason: &'a str,
        _timeout: f64,
        _lock: f64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, DbError>> + 'a>> {
        Box::pin(async { Ok(true) })
    }
}
