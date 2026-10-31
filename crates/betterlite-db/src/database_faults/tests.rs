#![cfg(test)]

use super::reconnect::{DbReconnect, RetryPolicy, call_with_db_reconnect_retry};
use super::*;
use std::future::{Future, ready};
use std::io::{self, Write};
use std::pin::Pin;
use std::str;
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
    assert!(classify(ErrorType::Deadline).connection);
    assert!(!classify(ErrorType::Prisma).connection);
}
#[test]
fn infrastructure_data_exclusions() {
    assert!(!classify(ErrorType::UniqueViolation).infrastructure);
    assert!(classify(ErrorType::Prisma).infrastructure);
}
#[test]
fn base_data_exact_type() {
    assert_eq!(error(ErrorType::Data).error_type, ErrorType::Data);
    assert_ne!(error(ErrorType::RawQuery).error_type, ErrorType::Data);
}
#[test]
fn p1001_base_data() {
    let mut value = error(ErrorType::Data);
    value.message = "can't reach database server".to_owned();
    assert_eq!(value.error_type, ErrorType::Data);
}

#[test]
fn unique_violation_uses_meta_sqlstate() {
    let mut value = error(ErrorType::UniqueViolation);
    value.meta_code = Some("23505".to_owned());
    assert_eq!(postgres_sqlstate(&value).as_deref(), Some("23505"));
}

#[rstest::rstest]
#[case::engine_connection(ErrorType::EngineConnection)]
#[case::client_not_connected(ErrorType::ClientNotConnected)]
#[case::http_client_closed(ErrorType::HttpClientClosed)]
fn prisma_subclasses_cover_all_predicates(#[case] error_type: ErrorType) {
    let classes = classify(error_type);
    assert!(classes.prisma);
    assert!(classes.infrastructure);
    assert!(!classify(ErrorType::Deadline).infrastructure);
}

#[test]
fn deadline_and_engine_transport_are_distinct() {
    assert!(!is_transport_error(&error(ErrorType::Deadline)));
    let mut engine = error(ErrorType::EngineConnection);
    assert!(!is_transport_error(&engine));
    engine.message = "connection refused".to_owned();
    assert!(is_transport_error(&engine));
}
#[test]
fn transport_keyword_matrix() {
    let mut value = error(ErrorType::Prisma);
    value.message = "NETWORK IS UNREACHABLE".to_owned();
    assert!(is_transport_error(&value));
    assert!(is_transport_error(&error(ErrorType::HttpClientClosed)));
}
#[test]
fn unrelated_message_not_transport() {
    let mut value = error(ErrorType::Prisma);
    value.message = "query rejected".to_owned();
    assert!(!is_transport_error(&value));
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
    assert!(!is_permanent_database_fault(&error(ErrorType::Connect)));
}
#[test]
fn exception_classes_only() {
    assert!(!classify(ErrorType::Other).prisma);
}

#[derive(Clone, Default)]
struct Effects(Arc<Mutex<Vec<String>>>);

impl Write for Effects {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .push(
                str::from_utf8(buf)
                    .map_err(io::Error::other)?
                    .trim()
                    .to_owned(),
            );
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ScriptedClient {
    effects: Effects,
    reconnect: Result<bool, DbError>,
    timeouts: (Option<f64>, Option<f64>),
}

impl DbReconnect for ScriptedClient {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        self.timeouts.0
    }
    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        self.timeouts.1
    }
    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        self.effects.0.lock().unwrap().push(format!(
            "reconnect {reason} {timeout_seconds} {lock_timeout_seconds}"
        ));
        let result = self.reconnect.clone();
        Box::pin(async move { result })
    }
}

const POLICY: RetryPolicy<'static> = RetryPolicy {
    reason: "read",
    retry_safe_error_types: None,
    timeout_seconds: None,
    lock_timeout_seconds: None,
};

async fn scripted_retry(
    reads: Vec<Result<i32, DbError>>,
    reconnect: Option<Result<bool, DbError>>,
    attributes: (Option<f64>, Option<f64>),
    policy: RetryPolicy<'_>,
) -> (Result<i32, DbError>, Vec<String>) {
    use tracing::instrument::WithSubscriber as _;
    let effects = Effects::default();
    let writer = effects.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_level(false)
        .with_writer(move || writer.clone())
        .finish();
    let mut client = reconnect.map(|outcome| ScriptedClient {
        effects: effects.clone(),
        reconnect: outcome,
        timeouts: attributes,
    });
    let mut outcomes = reads.into_iter();
    let result = call_with_db_reconnect_retry(
        client.as_mut(),
        || {
            effects.0.lock().unwrap().push("read".to_owned());
            ready(outcomes.next().expect("unexpected replay"))
        },
        policy,
    )
    .with_subscriber(subscriber)
    .await;
    let recorded = effects.0.lock().unwrap().clone();
    (result, recorded)
}

fn transport_error() -> DbError {
    let mut value = error(ErrorType::Connect);
    value.message = "connection refused".to_owned();
    value
}

const FIRST_WARNING: &str = "DB transport error on read; attempting reconnect-and-retry reason=\"read\" error=connection refused";

#[tokio::test]
async fn read_success() {
    let (result, effects) =
        scripted_retry(vec![Ok(7_i32)], Some(Ok(true)), (None, None), POLICY).await;
    assert_eq!(result, Ok(7_i32));
    assert_eq!(effects, ["read"]);
}

#[tokio::test]
async fn data_no_reconnect() {
    let first = error(ErrorType::Data);
    let (result, effects) = scripted_retry(
        vec![Err(first.clone()), Ok(7_i32)],
        Some(Ok(true)),
        (None, None),
        POLICY,
    )
    .await;
    assert_eq!(result, Err(first));
    assert_eq!(effects, ["read"]);
}

#[rstest::rstest]
#[case::unsafe_transport_no_replay(ErrorType::ReadTimeout)]
#[case::connect_timeout(ErrorType::ConnectTimeout)]
#[case::read_error(ErrorType::Read)]
#[tokio::test]
async fn unsafe_transport_no_replay(#[case] kind: ErrorType) {
    let first = error(kind);
    let mut retry_policy = POLICY;
    retry_policy.retry_safe_error_types = Some(&[ErrorType::Connect]);
    let (result, effects) = scripted_retry(
        vec![Err(first.clone())],
        Some(Ok(true)),
        (None, None),
        retry_policy,
    )
    .await;
    assert_eq!(result, Err(first));
    assert_eq!(effects, ["read"]);
}

#[tokio::test]
async fn prisma_allowlist_covers_subclasses() {
    let mut first = error(ErrorType::ClientNotConnected);
    first.message = "connection refused".to_owned();
    let mut retry_policy = POLICY;
    retry_policy.retry_safe_error_types = Some(&[ErrorType::Prisma]);
    let (result, effects) = scripted_retry(
        vec![Err(first), Ok(5_i32)],
        Some(Ok(true)),
        (None, None),
        retry_policy,
    )
    .await;
    assert_eq!(result, Ok(5_i32));
    assert_eq!(effects.len(), 4);
}

#[tokio::test]
async fn absent_reconnect() {
    let first = transport_error();
    let (result, effects) = scripted_retry(
        vec![Err(first.clone())],
        None,
        (Some(99.0_f64), Some(99.0_f64)),
        POLICY,
    )
    .await;
    assert_eq!(result, Err(first));
    assert_eq!(effects, ["read"]);
}

#[rstest::rstest]
#[case::explicit((Some(7.0_f64), Some(0.4_f64)), (Some(3.0_f64), Some(0.2_f64)), "reconnect read 7 0.4")]
#[case::attributes((None, None), (Some(3.0_f64), Some(0.2_f64)), "reconnect read 3 0.2")]
#[case::defaults((None, None), (None, None), "reconnect read 2 0.1")]
#[case::independent((Some(7.0_f64), None), (Some(3.0_f64), Some(0.2_f64)), "reconnect read 7 0.2")]
#[case::numeric_timeout_passthrough((Some(-1.0_f64), Some(0.0_f64)), (None, None), "reconnect read -1 0")]
#[tokio::test]
async fn timeout_argument_attribute_defaults(
    #[case] explicit: (Option<f64>, Option<f64>),
    #[case] attributes: (Option<f64>, Option<f64>),
    #[case] expected: &str,
) {
    let (result, effects) = scripted_retry(
        vec![Err(transport_error()), Ok(4_i32)],
        Some(Ok(true)),
        attributes,
        RetryPolicy {
            timeout_seconds: explicit.0,
            lock_timeout_seconds: explicit.1,
            ..POLICY
        },
    )
    .await;
    assert_eq!(result, Ok(4_i32));
    assert_eq!(effects, ["read", FIRST_WARNING, expected, "read"]);
}

#[tokio::test]
async fn reconnect_warning_arguments() {
    let (result, effects) = scripted_retry(
        vec![Err(transport_error()), Ok(7_i32)],
        Some(Ok(true)),
        (None, None),
        POLICY,
    )
    .await;
    assert_eq!(result, Ok(7_i32));
    assert_eq!(
        effects,
        ["read", FIRST_WARNING, "reconnect read 2 0.1", "read"]
    );
}

#[tokio::test]
async fn reconnect_false_preserves_first() {
    let first = transport_error();
    let (result, effects) = scripted_retry(
        vec![Err(first.clone())],
        Some(Ok(false)),
        (None, None),
        POLICY,
    )
    .await;
    assert_eq!(result, Err(first));
    assert_eq!(effects, ["read", FIRST_WARNING, "reconnect read 2 0.1"]);
}

#[tokio::test]
async fn reconnect_raise_chains_first() {
    let first = transport_error();
    let mut reconnect_error = error(ErrorType::Other);
    reconnect_error.message = "reconnect failed".to_owned();
    let (result, effects) = scripted_retry(
        vec![Err(first.clone())],
        Some(Err(reconnect_error.clone())),
        (None, None),
        POLICY,
    )
    .await;
    let expected = DbError {
        cause: Some(Box::new(reconnect_error)),
        ..first
    };
    assert_eq!(result, Err(expected));
    assert_eq!(
        effects,
        [
            "read",
            FIRST_WARNING,
            "reconnect read 2 0.1",
            "DB reconnect attempt raised; preserving original transport error reason=\"read\" reconnect_error=reconnect failed"
        ]
    );
}

#[rstest::rstest]
#[case::retry_success(Ok(9_i32))]
#[case::retry_failure_no_loop(Err(transport_error()))]
#[tokio::test]
async fn retry_success(#[case] second: Result<i32, DbError>) {
    let (result, effects) = scripted_retry(
        vec![Err(transport_error()), second.clone()],
        Some(Ok(true)),
        (None, None),
        POLICY,
    )
    .await;
    assert_eq!(result, second);
    assert_eq!(
        effects,
        ["read", FIRST_WARNING, "reconnect read 2 0.1", "read"]
    );
}

#[rstest::rstest]
#[case::connect(ErrorType::Connect)]
#[case::connect_timeout(ErrorType::ConnectTimeout)]
#[case::read(ErrorType::Read)]
#[case::read_timeout(ErrorType::ReadTimeout)]
fn transport_type_predicates(#[case] kind: ErrorType) {
    let value = error(kind);
    let classes = classify(kind);
    assert!(classes.connection);
    assert!(classes.infrastructure);
    assert!(is_transport_error(&value));
    assert!(!classes.prisma);
}

#[rstest::rstest]
#[case::data(ErrorType::Data)]
#[case::unique(ErrorType::UniqueViolation)]
fn prisma_subclass_transport_keywords(#[case] kind: ErrorType) {
    let mut value = error(kind);
    value.message = "connection refused".to_owned();
    assert!(is_transport_error(&value));
}

#[rstest::rstest]
#[case::exact_match(ErrorType::Connect, ErrorType::Connect, true)]
#[case::prisma_covers_client_not_connected(ErrorType::ClientNotConnected, ErrorType::Prisma, true)]
#[case::prisma_covers_unique_violation(ErrorType::UniqueViolation, ErrorType::Prisma, true)]
#[case::data_covers_unique_violation(ErrorType::UniqueViolation, ErrorType::Data, true)]
#[case::data_excludes_client_not_connected(ErrorType::ClientNotConnected, ErrorType::Data, false)]
#[case::connect_has_no_subclasses(ErrorType::ReadTimeout, ErrorType::Connect, false)]
fn subclass_membership(
    #[case] error_type: ErrorType,
    #[case] ancestor: ErrorType,
    #[case] expected: bool,
) {
    assert_eq!(is_error_type_or_subclass(error_type, ancestor), expected);
}

#[rstest::rstest]
#[case::too_long("PostgresError { code: \"123456\"", None)]
#[case::missing_quote("PostgresError { code: \"ABCDE", None)]
#[case::later_valid(
    "PostgresError { code: \"abcde\" PostgresError { code: \"12345\"",
    Some("12345")
)]
fn exact_connector_sqlstate(#[case] message: &str, #[case] expected: Option<&str>) {
    let mut value = error(ErrorType::Data);
    value.message = message.to_owned();
    assert_eq!(postgres_sqlstate(&value).as_deref(), expected);
}

#[rstest::rstest]
#[case::boolean("true")]
#[case::string("\"9\"")]
#[case::mock("{\"mock\":true}")]
#[test]
fn timeout_bool_string_fallback(#[case] input: &str) {
    let value: reconnect::Timeout = serde_json::from_str(input).unwrap();
    assert_eq!(value.coerce(2.0).to_bits(), 2.0_f64.to_bits());
    assert_eq!(value.coerce(0.1).to_bits(), 0.1_f64.to_bits());
}
