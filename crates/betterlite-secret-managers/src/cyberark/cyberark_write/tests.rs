#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::write;
use crate::cyberark::operation::OperationStatus;
use crate::cyberark::tests::{POLICY_ROUTE, Reply, TOKEN_HEADER, capture_logs, keyed};
use crate::cyberark::yaml_scalar::double_quoted;

const SECRET_ROUTE: &str = "POST http://127.0.0.1:8080/secrets/default/variable/db";

#[rstest]
#[case::plain("abc", "\"abc\"")]
#[case::slash("a/b", "\"a/b\"")]
#[case::quote("a\"b", "\"a\\\"b\"")]
#[case::backslash("a\\b", "\"a\\\\b\"")]
#[case::latin1("\u{e9}", "\"\\xE9\"")]
#[case::no_break_space("\u{a0}", "\"\\_\"")]
#[case::byte_order_mark("\u{feff}", "\"\\uFEFF\"")]
#[case::bmp("\u{100}\u{ffff}", "\"\\u0100\\uFFFF\"")]
#[case::astral("\u{1f600}", "\"\\U0001F600\"")]
#[case::controls(
    "\0\u{7}\u{8}\t\n\u{b}\u{c}\r\u{1b}\u{85}\u{2028}\u{2029}",
    "\"\\0\\a\\b\\t\\n\\v\\f\\r\\e\\N\\L\\P\""
)]
#[case::long_unbroken(&"a".repeat(100), &format!("\"{}\"", "a".repeat(100)))]
#[case::quotes_fold(&"x\"".repeat(50), &format!("\"{}\\\n  {}\"", "x\\\"".repeat(27), "x\\\"".repeat(23)))]
#[case::fold_after_quotes(&format!("{}{}", "a".repeat(70), "\"".repeat(20)), &format!("\"{}{}\\\n  {}\"", "a".repeat(70), "\\\"".repeat(6), "\\\"".repeat(14)))]
#[case::fold_after_astral(&format!("{}\u{1f600}b", "a".repeat(76)), &format!("\"{}\\U0001F600\\\n  b\"", "a".repeat(76)))]
#[case::fold_after_backslash(&format!("{}\\{}", "ab".repeat(40), "c".repeat(10)), &format!("\"{}\\\\\\\n  {}\"", "ab".repeat(40), "c".repeat(10)))]
#[case::trailing_space_never_folds(&format!("{} ", "a".repeat(80)), &format!("\"{} \"", "a".repeat(80)))]
#[case::fold_at_space(&["word"; 30].join(" "), &format!("\"{}\\\n  \\ {}\"", ["word"; 17].join(" "), ["word"; 13].join(" ")))]
fn the_policy_scalar_matches_pyyaml(#[case] name: &str, #[case] expected: &str) {
    assert_eq!(double_quoted(name), expected);
}

#[tokio::test(start_paused = true)]
async fn a_write_creates_the_policy_then_stores_and_caches_the_value() {
    let (manager, transport) = keyed(&[
        (POLICY_ROUTE, vec![Reply::Status(201, "")]),
        (SECRET_ROUTE, vec![Reply::Status(201, "")]),
    ]);
    let outcome = write(&manager, "db", &SecretString::from("s3cret")).await;
    assert_eq!(outcome.status, OperationStatus::Success);
    assert_eq!(outcome.message, "Secret db written successfully");
    let seen = transport.recorded();
    let routes: Vec<&str> = seen.iter().map(|request| request.route.as_str()).collect();
    assert_eq!(routes[1..], [POLICY_ROUTE, SECRET_ROUTE]);
    let policy = &seen[1];
    assert_eq!(policy.body.as_deref(), Some("- !variable \"db\"\n"));
    assert_eq!(policy.content_type, Some("application/x-yaml"));
    assert_eq!(policy.authorization.as_deref(), Some(TOKEN_HEADER));
    let value = &seen[2];
    assert_eq!(value.body.as_deref(), Some("s3cret"));
    assert_eq!(value.content_type, None);
    assert_eq!(manager.read("db").await.unwrap().expose_secret(), "s3cret");
    assert_eq!(transport.recorded().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn conflicts_retry_with_doubling_delays_and_stop_after_five_calls() {
    let (manager, transport) = keyed(&[
        (POLICY_ROUTE, vec![Reply::Status(409, "")]),
        (SECRET_ROUTE, vec![Reply::Status(201, "")]),
    ]);
    let outcome = write(&manager, "db", &SecretString::from("v")).await;
    assert_eq!(outcome.status, OperationStatus::Success);
    let policy_times: Vec<Duration> = transport
        .recorded()
        .into_iter()
        .filter(|request| request.route == POLICY_ROUTE)
        .map(|request| request.at)
        .collect();
    let expected = [0, 200, 600, 1400, 3000].map(Duration::from_millis);
    assert_eq!(policy_times, expected);
}

#[tokio::test(start_paused = true)]
async fn a_cleared_conflict_ends_the_retries() {
    let (manager, transport) = keyed(&[
        (
            POLICY_ROUTE,
            vec![
                Reply::Status(409, ""),
                Reply::Status(409, ""),
                Reply::Status(201, ""),
            ],
        ),
        (SECRET_ROUTE, vec![Reply::Status(201, "")]),
    ]);
    write(&manager, "db", &SecretString::from("v")).await;
    let calls = transport
        .routes_hit()
        .iter()
        .filter(|route| route.as_str() == POLICY_ROUTE)
        .count();
    assert_eq!(calls, 3);
}

#[rstest]
#[case::unprocessable(
    Reply::Status(422, ""),
    "DEBUG Variable db policy was rejected as unprocessable"
)]
#[case::created(Reply::Status(201, ""), "DEBUG Created policy entry for variable: db")]
#[case::not_found(Reply::Status(404, ""), "WARN Could not ensure variable exists: 404")]
#[case::server_error(Reply::Status(500, ""), "WARN Could not ensure variable exists: 500")]
#[case::connection(
    Reply::Failure,
    "WARN Error ensuring variable exists: CyberArk Conjur request failed"
)]
#[tokio::test(start_paused = true)]
async fn the_policy_outcome_sets_the_log_level_and_the_value_is_still_written(
    #[case] reply: Reply,
    #[case] logged: &str,
) {
    let (logs, _guard) = capture_logs();
    let (manager, transport) = keyed(&[
        (POLICY_ROUTE, vec![reply]),
        (SECRET_ROUTE, vec![Reply::Status(201, "")]),
    ]);
    let outcome = write(&manager, "db", &SecretString::from("v")).await;
    assert_eq!(outcome.status, OperationStatus::Success);
    assert!(logs.text().contains(logged), "{}", logs.text());
    assert_eq!(
        transport.routes_hit().last().map(String::as_str),
        Some(SECRET_ROUTE)
    );
}

#[tokio::test(start_paused = true)]
async fn an_unsafe_name_is_rejected_before_any_request() {
    let (manager, transport) = keyed(&[]);
    let outcome = write(&manager, "bad\u{1}name", &SecretString::from("v")).await;
    assert_eq!(outcome.status, OperationStatus::Error);
    assert!(outcome.message.contains("invalid secret name"));
    assert!(transport.recorded().is_empty());
}

#[rstest]
#[case::status(Reply::Status(500, ""))]
#[case::connection(Reply::Failure)]
#[tokio::test(start_paused = true)]
async fn a_failed_value_write_is_an_error_status_and_leaves_the_cache_alone(#[case] reply: Reply) {
    let (logs, _guard) = capture_logs();
    let (manager, _transport) = keyed(&[
        (POLICY_ROUTE, vec![Reply::Status(201, "")]),
        (SECRET_ROUTE, vec![reply]),
    ]);
    let outcome = write(&manager, "db", &SecretString::from("v")).await;
    assert_eq!(outcome.status, OperationStatus::Error);
    assert!(
        logs.text()
            .contains("ERROR Error writing secret to CyberArk Conjur")
    );
    assert!(manager.read("db").await.is_none());
}

#[tokio::test(start_paused = true)]
async fn concurrent_writes_load_policies_one_after_another() {
    let (manager, transport) = keyed(&[
        (POLICY_ROUTE, vec![Reply::Status(409, "")]),
        (
            "POST http://127.0.0.1:8080/secrets/default/variable/one",
            vec![Reply::Status(201, "")],
        ),
        (
            "POST http://127.0.0.1:8080/secrets/default/variable/two",
            vec![Reply::Status(201, "")],
        ),
    ]);
    let value = SecretString::from("v");
    let (first, other) = tokio::join!(
        write(&manager, "one", &value),
        write(&manager, "two", &value)
    );
    assert_eq!(first.status, OperationStatus::Success);
    assert_eq!(other.status, OperationStatus::Success);
    let bodies: Vec<String> = transport
        .recorded()
        .into_iter()
        .filter(|request| request.route == POLICY_ROUTE)
        .filter_map(|request| request.body)
        .collect();
    let (ones, twos) = bodies.split_at(5);
    assert!(ones.iter().all(|body| body == "- !variable \"one\"\n"));
    assert!(twos.iter().all(|body| body == "- !variable \"two\"\n"));
}
