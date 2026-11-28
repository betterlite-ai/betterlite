#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::panic::Location;
use std::str;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_reporting::bug_report::notice::NOTICE_PREFIX;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use rstest::rstest;
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::testing::ScriptedCatalog;
use super::{
    CaughtException, DisconnectRelease, FailureFacts, HttpFailure, ModelNameCatalog,
    ParallelLimiterHook, ParallelLimiterHooks, ProxyDiagnostics, READ_ONLY_RECOVERY_REASON,
    ReadOnlyWriterRecovery, RecoverySubmission, RedactError, TrackedRecovery,
    handle_exception_on_proxy, is_known_model, redact_deployment, release_parallel_slot,
};

fn prisma_failure(message: &str) -> DbError {
    DbError {
        error_type: ErrorType::Prisma,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

#[track_caller]
fn failure_facts<'caught>(
    message: &'caught str,
    status_code: Option<u16>,
    param: Option<&'caught str>,
    database_error: Option<&'caught DbError>,
) -> FailureFacts<'caught> {
    FailureFacts {
        message,
        exception_type: "Failure",
        status_code,
        param,
        database_error,
        location: Location::caller(),
    }
}

fn existing_proxy() -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "original message".to_owned(),
        error_type: "invalid_request_error".to_owned(),
        param: Some("purpose".to_owned()),
        code: ExceptionCode::Status(400),
        headers: BTreeMap::from([("x-other".to_owned(), "keep".to_owned())]),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

struct SilentDiagnostics;

impl ProxyDiagnostics for SilentDiagnostics {
    fn bug_report_notice(&self, _failure: &FailureFacts<'_>) -> Option<String> {
        None
    }
}

struct AskedFacts {
    status_code: Option<u16>,
    log_at_ask: String,
}

const DIAGNOSTIC_NOTICE: &str = "scripted bug report notice";

#[derive(Default)]
struct Audit {
    trail: Mutex<Vec<&'static str>>,
    asked: Mutex<Vec<AskedFacts>>,
}

struct RecordingDiagnostics {
    logs: LogBuffer,
    notice: bool,
    audit: Arc<Audit>,
}

impl ProxyDiagnostics for RecordingDiagnostics {
    fn bug_report_notice(&self, failure: &FailureFacts<'_>) -> Option<String> {
        self.audit
            .trail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("diagnostics");
        self.audit
            .asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(AskedFacts {
                status_code: failure.status_code,
                log_at_ask: self.logs.contents(),
            });
        self.notice
            .then(|| format!("{NOTICE_PREFIX} {DIAGNOSTIC_NOTICE}"))
    }
}

struct RecordingRecovery {
    logs: LogBuffer,
    audit: Arc<Audit>,
    submitted: Mutex<Vec<String>>,
}

impl RecoverySubmission for RecordingRecovery {
    fn submit_read_only_recovery(&self) {
        self.audit
            .trail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("recovery");
        self.submitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.logs.contents());
    }
}

struct PendingWriter {
    gate: Semaphore,
    completed: AtomicBool,
    reasons: Mutex<Vec<String>>,
}

impl Default for PendingWriter {
    fn default() -> Self {
        Self {
            gate: Semaphore::new(0),
            completed: AtomicBool::new(false),
            reasons: Mutex::new(Vec::new()),
        }
    }
}

impl ReadOnlyWriterRecovery for PendingWriter {
    fn recreate_read_only_writer(&self, reason: &str) -> impl Future<Output = ()> + Send {
        self.reasons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(reason.to_owned());
        async move {
            let _permit = self.gate.acquire().await.expect("the gate never closes");
            self.completed.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("the v3 release failed")]
struct ReleaseFailed;

struct RecordingLimiter {
    gate: Semaphore,
    keys: Mutex<Vec<String>>,
    released: AtomicBool,
    fails: bool,
}

impl RecordingLimiter {
    fn new(fails: bool) -> Self {
        Self {
            gate: Semaphore::new(0),
            keys: Mutex::new(Vec::new()),
            released: AtomicBool::new(false),
            fails,
        }
    }
}

impl DisconnectRelease for RecordingLimiter {
    type Failure = ReleaseFailed;

    fn release_on_disconnect(
        &self,
        principal: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<(), ReleaseFailed>> + Send {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(principal_key(principal));
        async move {
            if self.fails {
                Err(ReleaseFailed)
            } else {
                let _permit = self.gate.acquire().await.expect("the gate never closes");
                self.released.store(true, Ordering::SeqCst);
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy)]
enum HookKind {
    Missing,
    Older,
    V3,
}

struct ScriptedHooks {
    limiter: RecordingLimiter,
    kind: HookKind,
}

impl ParallelLimiterHooks for ScriptedHooks {
    type Handler = RecordingLimiter;

    fn parallel_request_limiter(&self) -> Option<ParallelLimiterHook<'_, RecordingLimiter>> {
        match self.kind {
            HookKind::Missing => None,
            HookKind::Older => Some(ParallelLimiterHook::Older),
            HookKind::V3 => Some(ParallelLimiterHook::V3(&self.limiter)),
        }
    }
}

struct FailingCatalog;

impl ModelNameCatalog for FailingCatalog {
    fn model_names(&self) -> Result<Vec<String>, LookupError> {
        Err(LookupError::InvalidRow { index: 0 })
    }
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
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

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::ERROR)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

fn recording_diagnostics(logs: &LogBuffer, notice: bool) -> (RecordingDiagnostics, Arc<Audit>) {
    let audit = Arc::new(Audit::default());
    let diagnostics = RecordingDiagnostics {
        logs: logs.clone(),
        notice,
        audit: Arc::clone(&audit),
    };
    (diagnostics, audit)
}

fn principal(api_key: &str) -> UserAPIKeyAuth {
    let fields = BTreeMap::from([("api_key".to_owned(), JsonData::String(api_key.to_owned()))]);
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the principal builds")
}

#[derive(Deserialize)]
struct KeyFacts {
    api_key: String,
}

fn principal_key(principal: &UserAPIKeyAuth) -> String {
    let value = serde_json::to_value(principal).expect("the principal serializes");
    serde_json::from_value::<KeyFacts>(value)
        .expect("the principal names its key")
        .api_key
}

fn deployment(source: &str) -> JsonObject {
    serde_json::from_str(source).expect("a deployment object")
}

#[rstest]
#[case::http_missing(
    CaughtException::Http(HttpFailure { status_code: 404, detail: "missing".to_owned() }),
    "missing",
    None,
    "404"
)]
#[case::http_server_error(
    CaughtException::Http(HttpFailure { status_code: 503, detail: "unavailable".to_owned() }),
    "unavailable",
    None,
    "503"
)]
#[case::failure_without_status(
    CaughtException::Failure(failure_facts("boom", None, None, None)),
    "boom",
    None,
    "500"
)]
#[case::failure_with_status_and_param(
    CaughtException::Failure(failure_facts("boom", Some(429), Some("file"), None)),
    "boom",
    Some("file"),
    "429"
)]
#[case::failure_stringified_param(
    CaughtException::Failure(failure_facts("boom", None, Some("None"), None)),
    "boom",
    None,
    "500"
)]
fn proxy_error_records(
    #[case] caught: CaughtException<'static>,
    #[case] message: &str,
    #[case] param: Option<&str>,
    #[case] code: &str,
) {
    let record = handle_exception_on_proxy(caught, None, &SilentDiagnostics, None);

    assert_eq!(record.message, message);
    assert_eq!(record.error_type, "internal_server_error");
    assert_eq!(record.param.as_deref(), param);
    assert_eq!(record.code, code);
    assert_eq!(record.openai_code.as_deref(), Some(code));
    assert!(record.headers.is_empty());

    let existing = existing_proxy();
    let expected = existing.clone();
    let preserved = handle_exception_on_proxy(
        CaughtException::Proxy(existing),
        None,
        &SilentDiagnostics,
        None,
    );

    assert_eq!(preserved, expected);
    assert_eq!(preserved.error_type, "invalid_request_error");
    assert_eq!(preserved.param.as_deref(), Some("purpose"));
    assert_eq!(preserved.code, "400");
    assert_eq!(
        preserved.headers.get("x-other").map(String::as_str),
        Some("keep")
    );
}

#[derive(Clone, Copy)]
enum CallIdCase {
    Http,
    Failure,
    Proxy(&'static [(&'static str, &'static str)]),
}

fn call_id_exception(case: CallIdCase) -> CaughtException<'static> {
    match case {
        CallIdCase::Http => CaughtException::Http(HttpFailure {
            status_code: 404,
            detail: "missing".to_owned(),
        }),
        CallIdCase::Failure => CaughtException::Failure(failure_facts("boom", None, None, None)),
        CallIdCase::Proxy(headers) => {
            CaughtException::Proxy(ProxyException::new(ProxyExceptionInit {
                message: "original".to_owned(),
                error_type: "invalid_request_error".to_owned(),
                param: None,
                code: ExceptionCode::Status(400),
                headers: headers
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect(),
                openai_code: None,
                provider_specific_fields: BTreeMap::new(),
            }))
        }
    }
}

#[rstest]
#[case::http_with_id(CallIdCase::Http, Some("new-id"), &[("x-litellm-call-id", "new-id")])]
#[case::http_without_id(CallIdCase::Http, None, &[])]
#[case::failure_with_id(CallIdCase::Failure, Some("new-id"), &[("x-litellm-call-id", "new-id")])]
#[case::proxy_keeps_existing(
    CallIdCase::Proxy(&[("x-litellm-call-id", "original"), ("x-other", "keep")]),
    Some("new-id"),
    &[("x-litellm-call-id", "original"), ("x-other", "keep")]
)]
#[case::proxy_adds_when_absent(
    CallIdCase::Proxy(&[("x-other", "keep")]),
    Some("new-id"),
    &[("x-litellm-call-id", "new-id"), ("x-other", "keep")]
)]
#[case::proxy_without_supplied_id(
    CallIdCase::Proxy(&[("x-litellm-call-id", "original")]),
    None,
    &[("x-litellm-call-id", "original")]
)]
fn call_id_precedence(
    #[case] case: CallIdCase,
    #[case] litellm_call_id: Option<&str>,
    #[case] expected: &[(&str, &str)],
) {
    let record = handle_exception_on_proxy(
        call_id_exception(case),
        litellm_call_id,
        &SilentDiagnostics,
        None,
    );
    let expected_headers: BTreeMap<String, String> = expected
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();

    assert_eq!(record.headers, expected_headers);
}

fn submitted_recovery(
    database: Option<&DbError>,
) -> (TaskTracker, Arc<PendingWriter>, ProxyException) {
    let tracker = TaskTracker::new();
    let writer = Arc::new(PendingWriter::default());
    let recovery = TrackedRecovery {
        tracker: tracker.clone(),
        writer: Arc::clone(&writer),
    };
    let facts = failure_facts("boom", None, None, database);
    let record = handle_exception_on_proxy(
        CaughtException::Failure(facts),
        None,
        &SilentDiagnostics,
        Some(&recovery),
    );
    (tracker, writer, record)
}

fn assert_recovery_reasons(writer: &PendingWriter, expected: bool) {
    let reasons = writer
        .reasons
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if expected {
        assert_eq!(reasons, [READ_ONLY_RECOVERY_REASON.to_owned()]);
    } else {
        assert!(reasons.is_empty());
    }
}

#[derive(Clone, Copy)]
enum RecoveryInterface {
    Present,
    Missing,
}

fn recovery_without_an_interface(database: Option<&DbError>) -> ProxyException {
    handle_exception_on_proxy(
        CaughtException::Failure(failure_facts("boom", None, None, database)),
        None,
        &SilentDiagnostics,
        None,
    )
}

#[rstest]
#[case::read_only(
    Some(prisma_failure("read-only transaction")),
    RecoveryInterface::Present,
    true
)]
#[case::read_only_without_an_interface(
    Some(prisma_failure("read-only transaction")),
    RecoveryInterface::Missing,
    false
)]
#[case::other_failure(
    Some(prisma_failure("permission denied for the write")),
    RecoveryInterface::Present,
    false
)]
#[case::no_database(None, RecoveryInterface::Present, false)]
#[tokio::test]
async fn recovery_submission(
    #[case] database: Option<DbError>,
    #[case] interface: RecoveryInterface,
    #[case] expected: bool,
) {
    if matches!(interface, RecoveryInterface::Missing) {
        let record = recovery_without_an_interface(database.as_ref());
        assert_eq!(
            record.code, "500",
            "a read-only failure without a recovery interface still maps"
        );
        return;
    }

    let (tracker, writer, record) = submitted_recovery(database.as_ref());

    assert_eq!(record.code, "500");
    assert_eq!(
        tracker.len(),
        usize::from(expected),
        "exactly one recovery is submitted for a read-only transaction with an interface"
    );
    assert!(
        !writer.completed.load(Ordering::SeqCst),
        "the mapping waits for the recovery"
    );
    if expected {
        writer.gate.add_permits(1);
    }
    tracker.close();
    tracker.wait().await;
    assert_eq!(writer.completed.load(Ordering::SeqCst), expected);
    assert_recovery_reasons(&writer, expected);
}

#[test]
fn diagnostics_order() {
    notice_logs_before_the_message_is_stripped();
    status_failure_skips_the_notice();
    recovery_precedes_the_diagnostics();
    existing_proxy_bypasses_the_diagnostics();
    http_failure_bypasses_the_diagnostics();
}

fn notice_logs_before_the_message_is_stripped() {
    let (logs, _guard) = capture_logs();
    let (diagnostics, audit) = recording_diagnostics(&logs, true);
    let message = format!("boom\n{NOTICE_PREFIX} filed later");

    let record = handle_exception_on_proxy(
        CaughtException::Failure(failure_facts(&message, None, None, None)),
        None,
        &diagnostics,
        None,
    );

    assert_eq!(record.message, "boom");
    assert_eq!(audit.asked.lock().unwrap().len(), 1);
    let log_at_ask = audit
        .asked
        .lock()
        .unwrap()
        .first()
        .expect("the service was asked")
        .log_at_ask
        .clone();
    assert!(
        log_at_ask.contains(NOTICE_PREFIX),
        "the unstripped message is logged before the service is asked"
    );
    assert!(logs.contents().contains(DIAGNOSTIC_NOTICE));
    assert_eq!(*audit.trail.lock().unwrap(), ["diagnostics"]);
}

fn status_failure_skips_the_notice() {
    let (logs, _guard) = capture_logs();
    let (diagnostics, audit) = recording_diagnostics(&logs, false);

    let record = handle_exception_on_proxy(
        CaughtException::Failure(failure_facts("boom", Some(429), None, None)),
        None,
        &diagnostics,
        None,
    );

    assert_eq!(record.code, "429");
    let asked = audit
        .asked
        .lock()
        .unwrap()
        .first()
        .expect("the service was asked")
        .status_code;
    assert_eq!(asked, Some(429));
    assert!(!logs.contents().contains(DIAGNOSTIC_NOTICE));
}

fn recovery_precedes_the_diagnostics() {
    let (logs, _guard) = capture_logs();
    let (diagnostics, audit) = recording_diagnostics(&logs, false);
    let recovery = RecordingRecovery {
        logs,
        audit: Arc::clone(&audit),
        submitted: Mutex::new(Vec::new()),
    };
    let database = prisma_failure("read-only transaction");

    let record = handle_exception_on_proxy(
        CaughtException::Failure(failure_facts(
            "database write failed",
            None,
            None,
            Some(&database),
        )),
        None,
        &diagnostics,
        Some(&recovery),
    );

    assert_eq!(record.message, "database write failed");
    assert_eq!(recovery.submitted.lock().unwrap().len(), 1);
    let submitted = recovery
        .submitted
        .lock()
        .unwrap()
        .first()
        .expect("the recovery was submitted")
        .clone();
    assert!(
        submitted.contains("Exception: database write failed"),
        "the exception is logged before classification"
    );
    assert_eq!(*audit.trail.lock().unwrap(), ["recovery", "diagnostics"]);
}

fn existing_proxy_bypasses_the_diagnostics() {
    let (logs, _guard) = capture_logs();
    let (diagnostics, audit) = recording_diagnostics(&logs, true);
    let message = format!("boom\n{NOTICE_PREFIX} filed later");
    let proxy = ProxyException::new(ProxyExceptionInit {
        message,
        error_type: "invalid_request_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(400),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    });

    let record = handle_exception_on_proxy(
        CaughtException::Proxy(proxy.clone()),
        None,
        &diagnostics,
        None,
    );

    assert_eq!(record, proxy);
    assert!(audit.asked.lock().unwrap().is_empty());
    assert!(!logs.contents().contains(DIAGNOSTIC_NOTICE));
}

fn http_failure_bypasses_the_diagnostics() {
    let (logs, _guard) = capture_logs();
    let (diagnostics, audit) = recording_diagnostics(&logs, true);

    let record = handle_exception_on_proxy(
        CaughtException::Http(HttpFailure {
            status_code: 404,
            detail: "missing".to_owned(),
        }),
        None,
        &diagnostics,
        None,
    );

    assert_eq!(record.code, "404");
    assert_eq!(record.param, None);
    assert!(audit.asked.lock().unwrap().is_empty());
    assert!(!logs.contents().contains(DIAGNOSTIC_NOTICE));
}

#[rstest]
#[case::exact(Some("model_a"), Some(&["model_a", "model_a"][..]), true)]
#[case::case_mismatch(Some("Model_A"), Some(&["model_a", "model_a"][..]), false)]
#[case::provider_prefix(Some("openai/model_a"), Some(&["model_a", "model_a"][..]), false)]
#[case::other_name(Some("model_b"), Some(&["model_a", "model_a"][..]), false)]
#[case::absent_model(None, Some(&["model_a"][..]), false)]
#[case::absent_router(Some("model_a"), None, false)]
#[case::empty_catalog(Some("model_a"), Some(&[][..]), false)]
fn known_upload_model(
    #[case] model: Option<&str>,
    #[case] names: Option<&[&str]>,
    #[case] expected: bool,
) {
    let catalog = ScriptedCatalog {
        names: names
            .unwrap_or_default()
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        calls: AtomicUsize::new(0),
    };
    let router: &dyn ModelNameCatalog = &catalog;
    let known =
        is_known_model(model, names.is_some().then_some(router)).expect("the catalog answers");

    assert_eq!(known, expected);
    let lookups = usize::from(names.is_some() && model.is_some());
    assert_eq!(catalog.calls.load(Ordering::SeqCst), lookups);

    let error = is_known_model(Some("model_a"), Some(&FailingCatalog))
        .expect_err("the catalog failure propagates");
    assert!(matches!(error, LookupError::InvalidRow { index: 0 }));
}

type InferredProvider = Box<dyn Fn(&str) -> Option<String>>;

struct RealCatalog {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
    provider: InferredProvider,
}

impl RealCatalog {
    fn with_models(names: &[&str]) -> Self {
        let mut catalog = DeploymentCatalog::default();
        for name in names {
            let mut params = JsonObject::new();
            params.insert("model".to_owned(), Json::Text("openai/gpt-4o".to_owned()));
            catalog.models.push(
                DeploymentSpec::new(
                    (*name).to_owned(),
                    params,
                    JsonObject::new(),
                    JsonObject::new(),
                    String::new,
                )
                .expect("the fixture deployment builds"),
            );
        }
        Self {
            catalog,
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
            provider: Box::new(|_model| None),
        }
    }

    fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &*self.provider,
        }
    }
}

#[test]
fn catalog_model_names_come_from_get_model_names() {
    let catalog = RealCatalog::with_models(&["model_a", "model_a"]);
    let lookup = catalog.lookup();
    let router: &dyn ModelNameCatalog = &lookup;

    assert!(is_known_model(Some("model_a"), Some(router)).expect("the catalog answers"));
    assert!(!is_known_model(Some("Model_A"), Some(router)).expect("the catalog answers"));
    assert!(!is_known_model(Some("openai/model_a"), Some(router)).expect("the catalog answers"));
}

#[tokio::test]
async fn disconnect_limiter_hook() {
    missing_and_older_hooks_release_nothing().await;
    v3_hook_awaits_the_release();
    failing_release_propagates().await;
}

async fn missing_and_older_hooks_release_nothing() {
    let key_a = principal("key_a");
    let missing = ScriptedHooks {
        limiter: RecordingLimiter::new(false),
        kind: HookKind::Missing,
    };
    release_parallel_slot(&missing, &key_a)
        .await
        .expect("a missing hook releases nothing");
    assert!(missing.limiter.keys.lock().unwrap().is_empty());

    let older = ScriptedHooks {
        limiter: RecordingLimiter::new(false),
        kind: HookKind::Older,
    };
    release_parallel_slot(&older, &key_a)
        .await
        .expect("an older hook releases nothing");
    assert!(older.limiter.keys.lock().unwrap().is_empty());
}

fn v3_hook_awaits_the_release() {
    let key_a = principal("key_a");
    let hooks = ScriptedHooks {
        limiter: RecordingLimiter::new(false),
        kind: HookKind::V3,
    };

    let mut release = Box::pin(release_parallel_slot(&hooks, &key_a));
    let mut context = Context::from_waker(Waker::noop());
    assert!(
        matches!(release.as_mut().poll(&mut context), Poll::Pending),
        "the release future is awaited"
    );
    assert_eq!(*hooks.limiter.keys.lock().unwrap(), ["key_a".to_owned()]);
    assert!(!hooks.limiter.released.load(Ordering::SeqCst));

    hooks.limiter.gate.add_permits(1);
    assert!(
        matches!(release.as_mut().poll(&mut context), Poll::Ready(Ok(()))),
        "the release completes once the limiter releases"
    );
    assert!(hooks.limiter.released.load(Ordering::SeqCst));
    assert_eq!(*hooks.limiter.keys.lock().unwrap(), ["key_a".to_owned()]);
}

async fn failing_release_propagates() {
    let key_a = principal("key_a");
    let failing = ScriptedHooks {
        limiter: RecordingLimiter::new(true),
        kind: HookKind::V3,
    };

    let error = release_parallel_slot(&failing, &key_a)
        .await
        .expect_err("the failure propagates");

    assert_eq!(error, ReleaseFailed);
    assert_eq!(
        *failing.limiter.keys.lock().unwrap(),
        ["key_a".to_owned()],
        "the release is tried once"
    );
}

#[rstest]
#[case::credentials_removed(
    r#"{"model_name": "dep", "model_info": {"id": "dep-1", "api_key": "sk-outside"}, "litellm_params": {"model": "openai/gpt-4o", "temperature": 0.2, "api_key": "sk-secret-value", "client_secret": "client", "vertex_credentials": "vertex", "vertex_ai_credentials": "vertex-ai", "aws_access_key_id": "AKIA", "aws_secret_access_key": "secret"}}"#,
    &[],
    r#"{"model_name": "dep", "model_info": {"id": "dep-1", "api_key": "sk-outside"}, "litellm_params": {"model": "openai/gpt-4o", "temperature": 0.2}}"#
)]
#[case::masker_applied(
    r#"{"litellm_params": {"model": "openai/gpt-4o", "api_secret": "sk-secret-7890", "nested": {"db_password": "hunter2"}}}"#,
    &[],
    r#"{"litellm_params": {"model": "openai/gpt-4o", "api_secret": "sk-s******7890", "nested": {"db_password": "*******"}}}"#
)]
#[case::caller_exclusion(
    r#"{"litellm_params": {"litellm_credential_name": "my-cred", "other_secret": "hidden-4321"}}"#,
    &["litellm_credential_name"],
    r#"{"litellm_params": {"litellm_credential_name": "my-cred", "other_secret": "hidd***4321"}}"#
)]
#[case::rate_limit_fields(
    r#"{"litellm_params": {"default_api_key_tpm_limit": "100", "default_api_key_rpm_limit": "50"}}"#,
    &[],
    r#"{"litellm_params": {"default_api_key_tpm_limit": "100", "default_api_key_rpm_limit": "50"}}"#
)]
fn deployment_redaction(#[case] source: &str, #[case] excluded: &[&str], #[case] expected: &str) {
    let excluded_keys: Vec<String> = excluded.iter().map(|key| (*key).to_owned()).collect();

    let redacted =
        redact_deployment(deployment(source), &excluded_keys).expect("the deployment redacts");

    assert_eq!(redacted, deployment(expected));
    assert!(matches!(
        redact_deployment(deployment(r#"{"model_name": "dep"}"#), &[]),
        Err(RedactError::MissingParams)
    ));
    assert!(matches!(
        redact_deployment(deployment(r#"{"litellm_params": "text"}"#), &[]),
        Err(RedactError::MissingParams)
    ));
}
