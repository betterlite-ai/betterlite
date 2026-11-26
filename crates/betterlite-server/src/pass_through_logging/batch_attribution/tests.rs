#![cfg(test)]

use std::convert::Infallible;
use std::future::pending;
use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_auth::user_roles::UserRole;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::batches::records::BatchObject;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::{
    is_collection_route, log_batch_registration_result, optional_str, request_tags_from_metadata,
    spawn_batch_registration,
};
use crate::pass_through_logging::anthropic_batch::{
    AnthropicBatchLogging, BatchRegistrant, BatchRegistration, ManagedBatchRegistry, RegistryError,
    ResponseIds,
};
use crate::pass_through_logging::background_tasks::BackgroundTasks;
use crate::pass_through_logging::vertex_batch_logging::VertexBatchLogging;

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
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

fn tags(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
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
    assert_eq!(
        request_tags_from_metadata(&request),
        Some(tags(&["a", "b"]))
    );
}

#[test]
fn batch_request_tags_fall_back_to_the_key_tags() {
    let request = metadata(r#"{"tags": [1, 2], "user_api_key_auth_metadata": {"tags": ["key"]}}"#);
    assert_eq!(request_tags_from_metadata(&request), Some(tags(&["key"])));
    let absent = metadata(r#"{"user_api_key_auth_metadata": {"tags": ["key"]}}"#);
    assert_eq!(request_tags_from_metadata(&absent), Some(tags(&["key"])));
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
    assert_eq!(
        request_tags_from_metadata(&request),
        Some(tags(&["ab", "c"]))
    );
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
        Ok(Err("store failed")),
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
        Ok(Err("store failed")),
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

struct GatedRegistry {
    started: mpsc::Sender<()>,
    release: Arc<Notify>,
}

impl ManagedBatchRegistry for GatedRegistry {
    fn store_unified_object_id(
        &self,
        _request: BatchRegistration,
    ) -> BoxFuture<'static, Result<(), RegistryError>> {
        let started = self.started.clone();
        let release = Arc::clone(&self.release);
        Box::pin(async move {
            let _sent = started.send(()).await;
            release.notified().await;
            Ok(())
        })
    }
}

struct PanickingRegistry;

impl ManagedBatchRegistry for PanickingRegistry {
    fn store_unified_object_id(
        &self,
        _request: BatchRegistration,
    ) -> BoxFuture<'static, Result<(), RegistryError>> {
        Box::pin(async { panic!("the registry write panicked") })
    }
}

struct StalledRegistry;

impl ManagedBatchRegistry for StalledRegistry {
    fn store_unified_object_id(
        &self,
        _request: BatchRegistration,
    ) -> BoxFuture<'static, Result<(), RegistryError>> {
        Box::pin(async { pending::<Result<(), RegistryError>>().await })
    }
}

fn gated_registry() -> (
    Arc<dyn ManagedBatchRegistry>,
    mpsc::Receiver<()>,
    Arc<Notify>,
) {
    let (started, received) = mpsc::channel(1);
    let release = Arc::new(Notify::new());
    let registry: Arc<dyn ManagedBatchRegistry> = Arc::new(GatedRegistry {
        started,
        release: Arc::clone(&release),
    });
    (registry, received, release)
}

fn batch_object() -> BatchObject {
    serde_json::from_str(
        r#"{"id":"batch-1","completion_window":"24h","created_at":0,"endpoint":"/v1/chat/completions","input_file_id":"file-1","object":"batch","status":"in_progress"}"#,
    )
    .expect("a valid batch object")
}

fn registration(is_batch_create: bool) -> BatchRegistration {
    BatchRegistration {
        unified_object_id: "unified-1".to_owned(),
        model_object_id: "batch-1".to_owned(),
        batch_object: batch_object(),
        file_purpose: "batch".to_owned(),
        registrant: BatchRegistrant {
            user_id: None,
            api_key: None,
            team_id: None,
            team_alias: None,
            user_role: UserRole::Customer,
            user_email: None,
            max_budget: None,
            spend: Decimal::ZERO,
            models: Vec::new(),
            tpm_limit: None,
            rpm_limit: None,
            budget_duration: None,
            budget_reset_at: None,
            max_parallel_requests: None,
            allowed_model_region: None,
            metadata: JsonObject::new(),
            key_alias: None,
            permissions: JsonObject::new(),
            model_max_budget: JsonObject::new(),
            model_spend: JsonObject::new(),
        },
        request_tags: None,
        persist_attribution: is_batch_create,
        create_if_missing: is_batch_create,
    }
}

struct TestIds;

impl ResponseIds for TestIds {
    fn new_response_id(&self) -> String {
        "resp-1".to_owned()
    }
}

#[tokio::test]
async fn batch_registration_drain_waits_for_an_anthropic_create() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    let (registry, mut started, release) = gated_registry();
    let background = BackgroundTasks::default();
    let logging =
        AnthropicBatchLogging::new(None, Some(registry), Arc::new(TestIds), background.clone());
    logging.store_batch_managed_object("unified-1", batch_object(), "batch-1", &JsonObject::new());
    timeout(Duration::from_secs(5), started.recv())
        .await
        .expect("the registry write starts")
        .expect("the registry reports its start");
    assert!(!log.contents().contains("Stored Anthropic"));
    assert!(background.shutdown().now_or_never().is_none());
    release.notify_one();
    background.shutdown().await;
    let text = log.contents();
    assert!(text.contains(
        "Stored Anthropic batch managed object with unified_object_id=unified-1, batch_id=batch-1"
    ));
    assert!(!text.contains("cancelled at shutdown"));
}

#[tokio::test]
async fn batch_registration_drain_waits_for_a_vertex_read() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    let (registry, mut started, release) = gated_registry();
    let background = BackgroundTasks::default();
    let logging =
        VertexBatchLogging::new(None, Some(registry), Arc::new(TestIds), background.clone());
    logging.store_batch_managed_object("unified-1", batch_object(), &JsonObject::new(), false);
    timeout(Duration::from_secs(5), started.recv())
        .await
        .expect("the registry write starts")
        .expect("the registry reports its start");
    assert!(!log.contents().contains("Stored Vertex AI"));
    assert!(background.shutdown().now_or_never().is_none());
    release.notify_one();
    background.shutdown().await;
    let text = log.contents();
    assert!(text.contains(
        "Stored Vertex AI batch managed object with unified_object_id=unified-1, batch_id=batch-1"
    ));
    assert!(!text.contains("cancelled at shutdown"));
}

#[tokio::test]
async fn batch_registration_drain_warns_about_a_cancelled_create() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    let (registry, mut started, _release) = gated_registry();
    let background = BackgroundTasks::default();
    let handle = spawn_batch_registration(&background, &registry, "Anthropic", registration(true))
        .expect("a Tokio runtime runs the test");
    timeout(Duration::from_secs(5), started.recv())
        .await
        .expect("the registry write starts")
        .expect("the registry reports its start");
    handle.abort();
    let error = handle
        .await
        .expect_err("the registration task was cancelled");
    assert!(error.is_cancelled());
    assert!(log.contents().contains(
        "Anthropic batch registration cancelled at shutdown; its cost will not be tracked"
    ));
}

#[tokio::test]
async fn batch_registration_drain_warns_about_a_cancelled_read() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    let (registry, mut started, _release) = gated_registry();
    let background = BackgroundTasks::default();
    let handle = spawn_batch_registration(&background, &registry, "Vertex AI", registration(false))
        .expect("a Tokio runtime runs the test");
    timeout(Duration::from_secs(5), started.recv())
        .await
        .expect("the registry write starts")
        .expect("the registry reports its start");
    handle.abort();
    let error = handle
        .await
        .expect_err("the registration task was cancelled");
    assert!(error.is_cancelled());
    assert!(log.contents().contains(
        "Vertex AI batch registration cancelled at shutdown; its status and output file may be stale"
    ));
}

#[tokio::test]
async fn batch_registration_drain_warns_when_the_registry_panics() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    let registry: Arc<dyn ManagedBatchRegistry> = Arc::new(PanickingRegistry);
    let background = BackgroundTasks::default();
    let handle = spawn_batch_registration(&background, &registry, "Anthropic", registration(true))
        .expect("a Tokio runtime runs the test");
    let error = handle.await.expect_err("the registration task panicked");
    assert!(error.is_panic());
    let text = log.contents();
    assert!(text.contains(
        "Anthropic batch registration cancelled at shutdown; its cost will not be tracked"
    ));
    assert!(!text.contains("Failed to store Anthropic"));
}

#[tokio::test]
async fn batch_registration_drain_warns_about_an_unpolled_create() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    let (registry, mut started, _release) = gated_registry();
    let background = BackgroundTasks::default();
    let handle = spawn_batch_registration(&background, &registry, "Anthropic", registration(true))
        .expect("a Tokio runtime runs the test");
    handle.abort();
    let error = handle
        .await
        .expect_err("the registration task was cancelled");
    assert!(error.is_cancelled());
    assert_eq!(started.try_recv(), Err(TryRecvError::Empty));
    assert!(log.contents().contains(
        "Anthropic batch registration cancelled at shutdown; its cost will not be tracked"
    ));
}

#[tokio::test]
async fn batch_registration_drain_warns_about_an_unpolled_read() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    let (registry, mut started, _release) = gated_registry();
    let background = BackgroundTasks::default();
    let handle = spawn_batch_registration(&background, &registry, "Vertex AI", registration(false))
        .expect("a Tokio runtime runs the test");
    handle.abort();
    let error = handle
        .await
        .expect_err("the registration task was cancelled");
    assert!(error.is_cancelled());
    assert_eq!(started.try_recv(), Err(TryRecvError::Empty));
    assert!(log.contents().contains(
        "Vertex AI batch registration cancelled at shutdown; its status and output file may be stale"
    ));
}

#[tokio::test(start_paused = true)]
async fn batch_registration_drain_gives_up_on_a_stalled_registry() {
    let (log, _guard) = capture_logs(tracing::Level::INFO);
    let registry: Arc<dyn ManagedBatchRegistry> = Arc::new(StalledRegistry);
    let background = BackgroundTasks::default();
    let handle = spawn_batch_registration(&background, &registry, "Anthropic", registration(true))
        .expect("a Tokio runtime runs the test");
    timeout(Duration::from_secs(60), background.shutdown())
        .await
        .expect("the drain stops waiting for a stalled registration");
    handle
        .await
        .expect("the registration task reported its timeout");
    let text = log.contents();
    assert!(text.contains(
        "Failed to store Anthropic batch managed object with unified_object_id=unified-1, batch_id=batch-1; its cost will not be tracked: the batch managed object write did not finish within 30 seconds"
    ));
    assert!(!text.contains("cancelled at shutdown"));
}
