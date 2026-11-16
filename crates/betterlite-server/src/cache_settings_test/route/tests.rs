#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::str::from_utf8;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use axum::http::StatusCode;
use rstest::rstest;
use serde_json::Value;
use serde_json::json;
use tracing_subscriber::fmt::MakeWriter;

use crate::cache_settings_test::support;
use crate::cache_settings_test::support::FakeRedis;
use crate::cache_settings_test::support::SavedReply;

const ADMIN_REQUIRED: &str = "Admin access required to test cache settings";
const NOT_SUPPORTED: &str = "Only Redis cache type is currently supported for testing";
const MISSING_TARGET: &str = "Either 'host' or 'url' must be specified for redis.";
const CLUSTER_OWNED: &str = "cache type 'redis' with cluster mode is owned by B5640";
const SUCCESS_BODY: &str =
    r#"{"status":"success","message":"Redis connection test successful","error":null}"#;

fn redis_target(redis: &FakeRedis) -> Value {
    json!({ "type": "redis", "host": redis.host(), "port": redis.port() })
}

fn saved_target(redis: &FakeRedis) -> Value {
    let mut target = redis_target(redis);
    target["password"] = json!(support::sealed(support::SAVED_SECRET));
    target
}

fn captured(records: &Arc<Mutex<String>>) -> String {
    records
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn capture_logs() -> (Arc<Mutex<String>>, tracing::subscriber::DefaultGuard) {
    let records = Arc::new(Mutex::new(String::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(CaptureWriter(Arc::clone(&records)))
        .finish();
    (records, tracing::subscriber::set_default(subscriber))
}

async fn rejected(body: &str) -> String {
    let (status, text) = support::post(support::state(None), Some(support::admin()), body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    text
}

#[tokio::test]
async fn route_refuses_a_request_without_a_key() {
    let (saved_row, repository) = support::fake_saved(SavedReply::Missing);
    let (status, text) = support::post(support::state(repository), None, "{").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        text,
        r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#
    );
    assert_eq!(saved_row.calls(), 0);
}

#[tokio::test]
async fn route_refuses_a_non_admin_before_touching_the_target() {
    let redis = FakeRedis::start().await;
    let (saved_row, repository) = support::fake_saved(SavedReply::Stored(saved_target(&redis)));
    let body = support::request(redis_target(&redis));
    let (status, text) = support::post(
        support::state(repository),
        Some(support::non_admin()),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(text, format!(r#"{{"detail":"{ADMIN_REQUIRED}"}}"#));
    assert_eq!(saved_row.calls(), 0);
    assert_eq!(redis.connections(), 0);
}

#[tokio::test]
async fn route_admits_a_platform_admin() {
    let redis = FakeRedis::start().await;
    let mut target = redis_target(&redis);
    target["namespace"] = json!("cache");
    target["_backend"] = json!("none");
    let body = support::request(target);
    let (status, text) =
        support::post(support::state(None), Some(support::platform_admin()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text, SUCCESS_BODY);
}

#[rstest]
#[case::empty("")]
#[case::null("null")]
#[tokio::test]
async fn route_rejects_a_missing_body(#[case] body: &str) {
    let (saved_row, repository) = support::fake_saved(SavedReply::Missing);
    let (status, text) =
        support::post(support::state(repository), Some(support::admin()), body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        text,
        r#"{"detail":[{"type":"missing","loc":["body"],"msg":"Field required"}]}"#
    );
    assert_eq!(saved_row.calls(), 0);
}

#[rstest]
#[case::text(r#""redis""#)]
#[case::number("5")]
#[case::flag("true")]
#[case::list("[]")]
#[tokio::test]
async fn route_rejects_a_body_that_is_not_an_object(#[case] body: &str) {
    assert_eq!(
        rejected(body).await,
        r#"{"detail":[{"type":"model_attributes_type","loc":["body"],"msg":"Input should be a valid dictionary or object to extract fields from"}]}"#
    );
}

#[tokio::test]
async fn route_rejects_a_missing_cache_settings() {
    assert_eq!(
        rejected("{}").await,
        r#"{"detail":[{"type":"missing","loc":["body","cache_settings"],"msg":"Field required"}]}"#
    );
}

#[rstest]
#[case::null(r#"{"cache_settings":null}"#)]
#[case::list(r#"{"cache_settings":[]}"#)]
#[case::text(r#"{"cache_settings":"redis"}"#)]
#[case::number(r#"{"cache_settings":5}"#)]
#[tokio::test]
async fn route_rejects_cache_settings_that_are_not_an_object(#[case] body: &str) {
    assert_eq!(
        rejected(body).await,
        r#"{"detail":[{"type":"dict_type","loc":["body","cache_settings"],"msg":"Input should be a valid dictionary"}]}"#
    );
}

#[rstest]
#[case::broken("{")]
#[case::trailing(r#"{"cache_settings":{}} trailing"#)]
#[tokio::test]
async fn route_rejects_unreadable_json(#[case] body: &str) {
    assert_eq!(
        rejected(body).await,
        r#"{"detail":[{"type":"json_invalid","loc":["body"],"msg":"JSON decode error"}]}"#
    );
}

#[rstest]
#[case::absent("{}")]
#[case::null_type(r#"{"type":null}"#)]
#[case::local(r#"{"type":"local"}"#)]
#[case::redis_semantic(r#"{"type":"redis-semantic"}"#)]
#[case::numeric_type(r#"{"type":5}"#)]
#[tokio::test]
async fn route_tests_only_redis_caches(#[case] given: &str) {
    let redis = FakeRedis::start().await;
    let (saved_row, repository) = support::fake_saved(SavedReply::Stored(saved_target(&redis)));
    let body = format!(r#"{{"cache_settings":{given}}}"#);
    let (status, text) =
        support::post(support::state(repository), Some(support::admin()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        text,
        format!(r#"{{"status":"failed","message":"{NOT_SUPPORTED}","error":null}}"#)
    );
    assert_eq!(saved_row.calls(), 1);
    assert_eq!(redis.connections(), 0);
}

#[tokio::test]
async fn route_reports_a_construction_error() {
    let body = support::request(json!({"type": "redis"}));
    let (status, text) = support::post(support::state(None), Some(support::admin()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        support::answer(&text),
        json!({
            "status": "failed",
            "message": format!("Cache connection test failed: {MISSING_TARGET}"),
            "error": MISSING_TARGET,
        })
    );
}

#[tokio::test]
async fn route_reports_a_cluster_construction_error() {
    let body = support::request(json!({
        "type": "redis",
        "redis_startup_nodes": [{"host": "127.0.0.1", "port": 7000}],
    }));
    let (status, text) = support::post(support::state(None), Some(support::admin()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        support::answer(&text),
        json!({
            "status": "failed",
            "message": format!("Cache connection test failed: {CLUSTER_OWNED}"),
            "error": CLUSTER_OWNED,
        })
    );
}

#[tokio::test]
async fn route_reports_a_connection_error() {
    let port = support::closed_port().await;
    let body = support::request(json!({ "type": "redis", "host": "127.0.0.1", "port": port }));
    let (status, text) = support::post(support::state(None), Some(support::admin()), &body).await;
    assert_eq!(status, StatusCode::OK);
    let answer = support::answer(&text);
    assert_eq!(answer["status"], "failed");
    let error = answer["error"]
        .as_str()
        .expect("the failed answer carries the error text");
    assert!(!error.is_empty(), "{text}");
    assert_eq!(
        answer["message"].as_str(),
        Some(format!("Redis connection failed: {error}").as_str())
    );
}

#[tokio::test]
async fn route_uses_the_saved_password_for_the_same_target() {
    let redis = FakeRedis::start().await;
    let (_, repository) = support::fake_saved(SavedReply::Stored(saved_target(&redis)));
    let body = support::request(redis_target(&redis));
    let (status, text) =
        support::post(support::state(repository), Some(support::admin()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text, SUCCESS_BODY);
    let passwords = redis.auth_passwords();
    assert!(
        !passwords.is_empty(),
        "the saved password reached the redis"
    );
    assert!(
        passwords
            .iter()
            .all(|password| password == support::SAVED_SECRET)
    );
}

#[tokio::test]
async fn route_restores_the_saved_password_behind_the_redacted_marker() {
    let redis = FakeRedis::start().await;
    let (_, repository) = support::fake_saved(SavedReply::Stored(saved_target(&redis)));
    let mut target = redis_target(&redis);
    target["password"] = json!(support::REDACTED);
    let body = support::request(target);
    let (status, text) =
        support::post(support::state(repository), Some(support::admin()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text, SUCCESS_BODY);
    let passwords = redis.auth_passwords();
    assert!(
        !passwords.is_empty(),
        "the saved password reached the redis"
    );
    assert!(
        passwords
            .iter()
            .all(|password| password == support::SAVED_SECRET)
    );
}

#[tokio::test]
async fn route_prefers_a_typed_password_over_the_saved_one() {
    let redis = FakeRedis::start().await;
    let (_, repository) = support::fake_saved(SavedReply::Stored(saved_target(&redis)));
    let mut target = redis_target(&redis);
    target["password"] = json!(support::TYPED_SECRET);
    let body = support::request(target);
    let (_, text) = support::post(support::state(repository), Some(support::admin()), &body).await;
    assert_eq!(text, SUCCESS_BODY);
    assert!(!redis.sent_secret(support::SAVED_SECRET));
    let passwords = redis.auth_passwords();
    assert!(
        !passwords.is_empty(),
        "the typed password reached the redis"
    );
    assert!(
        passwords
            .iter()
            .all(|password| password == support::TYPED_SECRET)
    );
}

#[rstest]
#[case::empty_string(json!(""))]
#[case::null(Value::Null)]
#[tokio::test]
async fn route_clears_the_saved_password_with_an_empty_value(#[case] password: Value) {
    let redis = FakeRedis::start().await;
    let (_, repository) = support::fake_saved(SavedReply::Stored(saved_target(&redis)));
    let mut target = redis_target(&redis);
    target["password"] = password;
    let body = support::request(target);
    let (_, text) = support::post(support::state(repository), Some(support::admin()), &body).await;
    assert_eq!(text, SUCCESS_BODY);
    assert!(!redis.sent_secret(support::SAVED_SECRET));
}

#[rstest]
#[case::password("password")]
#[case::url("url")]
#[tokio::test]
async fn route_keeps_a_wrongly_typed_secret_out_of_the_answer(#[case] option: &str) {
    let redis = FakeRedis::start().await;
    let mut target = redis_target(&redis);
    target[option] = json!(12_345);
    let body = support::request(target);
    let (status, text) = support::post(support::state(None), Some(support::admin()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!text.contains("12345"), "{text}");
    let message = format!("the redis option \"{option}\" has the wrong type or value");
    assert_eq!(
        support::answer(&text),
        json!({
            "status": "failed",
            "message": format!("Cache connection test failed: {message}"),
            "error": message,
        })
    );
}

#[tokio::test]
async fn route_drops_the_redacted_password_without_a_saved_row() {
    let redis = FakeRedis::start().await;
    let mut target = redis_target(&redis);
    target["password"] = json!(support::REDACTED);
    let body = support::request(target);
    let (_, text) = support::post(support::state(None), Some(support::admin()), &body).await;
    assert_eq!(text, SUCCESS_BODY);
    assert!(redis.auth_passwords().is_empty());
    assert!(!redis.sent_secret(support::REDACTED));
}

#[derive(Clone, Copy, Debug)]
enum ChangedTarget {
    Host,
    Port,
    Url,
    StartupNodes,
    SentinelNodes,
    ServiceName,
}

fn move_saved_row_to_a_url(saved: &mut Value, port: u16) {
    let fields = saved.as_object_mut().expect("the saved row is an object");
    fields.remove("host");
    fields.remove("port");
    fields.insert("url".to_owned(), json!(format!("redis://127.0.0.1:{port}")));
}

fn saved_row_for(changed: ChangedTarget, port: u16, other_port: u16) -> Value {
    let mut saved = json!({
        "type": "redis",
        "host": "127.0.0.1",
        "port": port,
        "password": support::sealed(support::SAVED_SECRET),
    });
    match changed {
        ChangedTarget::Host => saved["host"] = json!("127.0.0.2"),
        ChangedTarget::Port => saved["port"] = json!(other_port),
        ChangedTarget::Url => move_saved_row_to_a_url(&mut saved, other_port),
        ChangedTarget::StartupNodes => {
            saved["redis_startup_nodes"] = json!([{"host": "127.0.0.2", "port": other_port}]);
        }
        ChangedTarget::SentinelNodes => {
            saved["sentinel_nodes"] = json!([["127.0.0.2", other_port]])
        }
        ChangedTarget::ServiceName => saved["service_name"] = json!("mymaster"),
    }
    saved
}

fn changed_body(changed: ChangedTarget, redis: &FakeRedis) -> String {
    match changed {
        ChangedTarget::Url => format!(
            r#"{{"cache_settings":{{"type":"redis","url":"redis://127.0.0.1:{}"}}}}"#,
            redis.port()
        ),
        ChangedTarget::Host
        | ChangedTarget::Port
        | ChangedTarget::StartupNodes
        | ChangedTarget::SentinelNodes
        | ChangedTarget::ServiceName => support::request(redis_target(redis)),
    }
}

#[rstest]
#[case::host(ChangedTarget::Host)]
#[case::port(ChangedTarget::Port)]
#[case::url(ChangedTarget::Url)]
#[case::startup_nodes(ChangedTarget::StartupNodes)]
#[case::sentinel_nodes(ChangedTarget::SentinelNodes)]
#[case::service_name(ChangedTarget::ServiceName)]
#[tokio::test]
async fn route_never_sends_the_saved_password_to_another_target(#[case] changed: ChangedTarget) {
    let redis = FakeRedis::start().await;
    let other_port = support::closed_port().await;
    let saved = saved_row_for(changed, redis.port(), other_port);
    let (saved_row, repository) = support::fake_saved(SavedReply::Stored(saved));
    let body = changed_body(changed, &redis);
    let (status, text) =
        support::post(support::state(repository), Some(support::admin()), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text, SUCCESS_BODY);
    assert_eq!(saved_row.calls(), 1);
    assert!(redis.connections() > 0, "the request target was reached");
    assert!(redis.auth_passwords().is_empty(), "{text}");
    assert!(!redis.sent_secret(support::SAVED_SECRET));
}

#[tokio::test]
async fn route_still_tests_when_the_saved_row_fails() {
    let redis = FakeRedis::start().await;
    let (saved_row, repository) = support::fake_saved(SavedReply::Failed);
    let body = support::request(redis_target(&redis));
    let (_, text) = support::post(support::state(repository), Some(support::admin()), &body).await;
    assert_eq!(text, SUCCESS_BODY);
    assert_eq!(saved_row.calls(), 1);
}

#[tokio::test]
async fn route_resolves_the_redis_environment_fallbacks() {
    let redis = FakeRedis::start().await;
    let environment = BTreeMap::from([
        ("REDIS_HOST".to_owned(), redis.host()),
        ("REDIS_PORT".to_owned(), redis.port().to_string()),
    ]);
    let body = support::request(json!({"type": "redis"}));
    let (status, text) = support::post(
        support::state_with_environment(environment),
        Some(support::admin()),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text, SUCCESS_BODY);
}

#[tokio::test]
async fn route_lets_an_explicit_null_url_override_the_redis_url_environment() {
    let redis = FakeRedis::start().await;
    let environment = BTreeMap::from([(
        "REDIS_URL".to_owned(),
        format!("redis://{}:{}", redis.host(), redis.port()),
    )]);
    let body = support::request(json!({"type": "redis", "url": null}));
    let (status, text) = support::post(
        support::state_with_environment(environment),
        Some(support::admin()),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(redis.connections(), 0);
    assert_eq!(
        support::answer(&text),
        json!({
            "status": "failed",
            "message": format!("Cache connection test failed: {MISSING_TARGET}"),
            "error": MISSING_TARGET,
        })
    );
}

#[tokio::test]
async fn route_logs_the_settings_redacted() {
    let (records, _captured) = capture_logs();
    let redis = FakeRedis::start().await;
    let mut target = redis_target(&redis);
    target["password"] = json!(support::TYPED_SECRET);
    let body = support::request(target);
    let (_, text) = support::post(support::state(None), Some(support::admin()), &body).await;
    assert_eq!(text, SUCCESS_BODY);
    let logged = captured(&records);
    assert!(
        logged.contains("Testing cache connection with settings"),
        "{logged}"
    );
    assert!(logged.contains(support::REDACTED), "{logged}");
    assert!(!logged.contains(support::TYPED_SECRET), "{logged}");
}

#[tokio::test]
async fn route_logs_the_settings_after_url_precedence() {
    let (records, _captured) = capture_logs();
    let redis = FakeRedis::start().await;
    let url = format!("redis://{}:{}", redis.host(), redis.port());
    let body = support::request(json!({
        "type": "redis",
        "url": url,
        "host": "unused.example",
        "port": 6399,
        "username": "someone",
        "password": support::TYPED_SECRET,
    }));
    let (_, text) = support::post(support::state(None), Some(support::admin()), &body).await;
    assert_eq!(text, SUCCESS_BODY);
    assert!(redis.auth_passwords().is_empty(), "{text}");
    assert!(!redis.sent_secret(support::TYPED_SECRET));
    let logged = captured(&records);
    assert!(!logged.contains("unused.example"), "{logged}");
    assert!(logged.contains(support::REDACTED), "{logged}");
    assert!(!logged.contains(support::TYPED_SECRET), "{logged}");
}

#[derive(Clone)]
struct CaptureWriter(Arc<Mutex<String>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let text =
            from_utf8(buffer).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_str(text);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        Self(Arc::clone(&self.0))
    }
}
