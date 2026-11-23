#![cfg(test)]

use std::convert::Infallible;
use std::future::pending;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio::task::JoinSet;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::{
    is_collection_route, log_batch_registration_result, optional_str, request_tags_from_metadata,
};

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the captured log lines are UTF-8")
    }
}

impl io::Write for LogBuffer {
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

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

fn metadata(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("a JSON object")
}

fn tags(items: &[&str]) -> Option<Vec<String>> {
    Some(items.iter().map(|item| (*item).to_owned()).collect())
}

#[test]
fn batch_optional_str_returns_json_strings_only() {
    assert_eq!(optional_str(&Json::Text("x".to_owned())), Some("x"));
    assert_eq!(optional_str(&Json::Null), None);
    assert_eq!(optional_str(&Json::Bool(true)), None);
    assert_eq!(
        optional_str(&Json::Number(serde_json::Number::from(5_u8))),
        None
    );
    assert_eq!(optional_str(&Json::List(Vec::new())), None);
    assert_eq!(optional_str(&Json::Object(IndexMap::new())), None);
}

#[test]
fn batch_is_collection_route_cuts_the_query_and_trailing_slashes() {
    assert!(is_collection_route(
        "/v1/messages/batches?beta=true",
        "/batches"
    ));
    assert!(is_collection_route("/v1/messages/batches///", "/batches"));
    assert!(is_collection_route("/v1/messages/batches", "/batches"));
    assert!(!is_collection_route("/v1/messages/batches/abc", "/batches"));
    assert!(!is_collection_route(
        "/v1/messages/batches/abc?x=1",
        "/batches"
    ));
    assert!(is_collection_route(
        "/v1/messages/batches?next=/batches/abc",
        "/batches"
    ));
}

#[test]
fn batch_request_tags_prefer_the_request_tags() {
    let request =
        metadata(r#"{"tags": ["a", "b"], "user_api_key_auth_metadata": {"tags": ["key"]}}"#);
    assert_eq!(request_tags_from_metadata(&request), tags(&["a", "b"]));
}

#[test]
fn batch_request_tags_fall_back_to_the_key_tags() {
    let request = metadata(r#"{"tags": [1, 2], "user_api_key_auth_metadata": {"tags": ["key"]}}"#);
    assert_eq!(request_tags_from_metadata(&request), tags(&["key"]));
    let absent = metadata(r#"{"user_api_key_auth_metadata": {"tags": ["key"]}}"#);
    assert_eq!(request_tags_from_metadata(&absent), tags(&["key"]));
}

#[test]
fn batch_request_tags_keep_an_empty_key_list() {
    let empty = metadata(r#"{"tags": [], "user_api_key_auth_metadata": {"tags": []}}"#);
    assert_eq!(request_tags_from_metadata(&empty), Some(Vec::new()));
    let numbers = metadata(r#"{"user_api_key_auth_metadata": {"tags": [1, 2]}}"#);
    assert_eq!(request_tags_from_metadata(&numbers), Some(Vec::new()));
    let not_a_list = metadata(r#"{"user_api_key_auth_metadata": {"tags": "key"}}"#);
    assert_eq!(request_tags_from_metadata(&not_a_list), None);
}

#[test]
fn batch_request_tags_strip_nul_and_drop_non_strings() {
    let request = metadata(r#"{"tags": ["a\u0000b", 7, "c\u0000"]}"#);
    assert_eq!(request_tags_from_metadata(&request), tags(&["ab", "c"]));
}

#[test]
fn batch_request_tags_return_none_without_a_source() {
    assert_eq!(
        request_tags_from_metadata(&metadata(r#"{"tags": []}"#)),
        None
    );
    assert_eq!(
        request_tags_from_metadata(&metadata(r#"{"tags": "a"}"#)),
        None
    );
    assert_eq!(
        request_tags_from_metadata(&metadata(r#"{"user_api_key_auth_metadata": "tags"}"#)),
        None
    );
    assert_eq!(request_tags_from_metadata(&metadata("{}")), None);
}

#[test]
fn batch_log_registration_reports_a_stored_object() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    log_batch_registration_result::<Infallible>(
        Ok(Ok(())),
        "Anthropic",
        "unified-1",
        "batch-1",
        true,
    );
    let text = log.contents();
    assert!(text.contains(
        "Stored Anthropic batch managed object with unified_object_id=unified-1, batch_id=batch-1"
    ));
}

#[test]
fn batch_log_registration_reports_a_failed_create() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    log_batch_registration_result(
        Ok::<(), &str>(Err("store failed")),
        "Anthropic",
        "unified-1",
        "batch-1",
        true,
    );
    let text = log.contents();
    assert!(text.contains(
        "Failed to store Anthropic batch managed object with unified_object_id=unified-1, batch_id=batch-1; its cost will not be tracked: store failed"
    ));
}

#[test]
fn batch_log_registration_reports_a_stale_object() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    log_batch_registration_result(
        Ok::<(), &str>(Err("store failed")),
        "Anthropic",
        "unified-1",
        "batch-1",
        false,
    );
    let text = log.contents();
    assert!(text.contains("; its status and output file may be stale: store failed"));
}

#[tokio::test]
async fn batch_log_registration_reports_a_cancelled_task() {
    let mut tasks = JoinSet::new();
    let handle = tasks.spawn(async { pending::<()>().await });
    handle.abort();
    let finished = tasks
        .join_next()
        .await
        .expect("the task ends")
        .expect_err("the task was cancelled");
    assert!(finished.is_cancelled());
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    log_batch_registration_result::<Infallible>(
        Err(finished),
        "Anthropic",
        "unified-1",
        "batch-1",
        true,
    );
    let text = log.contents();
    assert!(text.contains("; its cost will not be tracked: None"));
}

#[tokio::test]
async fn batch_log_registration_reports_a_panicked_task() {
    let mut tasks = JoinSet::new();
    tasks.spawn(async { panic!("store crashed") });
    let finished = tasks
        .join_next()
        .await
        .expect("the task ends")
        .expect_err("the task panicked");
    assert!(finished.is_panic());
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    log_batch_registration_result::<Infallible>(
        Err(finished),
        "Anthropic",
        "unified-1",
        "batch-1",
        false,
    );
    let text = log.contents();
    assert!(text.contains("; its status and output file may be stale: "));
    assert!(text.contains("store crashed"));
}
