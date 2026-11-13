#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use http::HeaderMap;
use rstest::rstest;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::callback_logger::CallbackLogger as _;
use crate::dynamic_logger_cache::DynamicLoggerCache;
use crate::fields::payload::StandardLoggingPayload;
use crate::generic_api_logger::generic_logger::retry::RetrySleeper;
use crate::newrelic_metrics::NewRelicMetricsError;
use crate::newrelic_metrics::logger::{NewRelicDeps, NewRelicMetricsLogger};
use crate::newrelic_metrics::region::EU_METRIC_ENDPOINT;
use crate::newrelic_metrics::transport::{NewRelicPost, NewRelicSendError, NewRelicTransport};

use super::{
    NewRelicTeamHandlerError, newrelic_credentials, newrelic_credentials_passed,
    newrelic_logger_for_request,
};

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.0,
    "completionStartTime": 1704067200.5,
    "response_time": 0.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "model_group": "production",
    "custom_llm_provider": "openai",
    "api_base": "https://api.openai.com",
    "metadata": {
        "user_api_key_team_id": "team-1",
        "user_api_key_team_alias": "Team One"
    },
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("a valid standard logging payload")
}

#[derive(Default)]
struct CapturingTransport {
    requests: Mutex<Vec<(&'static str, HeaderMap)>>,
}

#[async_trait]
impl NewRelicTransport for CapturingTransport {
    async fn post(&self, request: NewRelicPost) -> Result<u16, NewRelicSendError> {
        self.requests
            .lock()
            .expect("the request lock is not poisoned")
            .push((request.url, request.headers));
        Ok(202)
    }
}

struct NoopSleeper;

#[async_trait]
impl RetrySleeper for NoopSleeper {
    async fn sleep(&self, _duration: Duration) {}
}

struct Fixture {
    cache: DynamicLoggerCache<Arc<NewRelicMetricsLogger>>,
    builds: Arc<AtomicUsize>,
    transport: Arc<CapturingTransport>,
    tasks: TaskTracker,
}

fn fixture() -> Fixture {
    Fixture {
        cache: DynamicLoggerCache::new(
            Arc::new(|_logger: Arc<NewRelicMetricsLogger>| {}),
            Arc::new(|| 0.0_f64),
        ),
        builds: Arc::new(AtomicUsize::new(0)),
        transport: Arc::new(CapturingTransport::default()),
        tasks: TaskTracker::new(),
    }
}

impl Fixture {
    fn get(
        &self,
        params: &BTreeMap<String, String>,
    ) -> Result<Arc<NewRelicMetricsLogger>, NewRelicTeamHandlerError> {
        let counter = Arc::clone(&self.builds);
        newrelic_logger_for_request(
            params,
            &self.cache,
            NewRelicDeps {
                transport: Arc::<CapturingTransport>::clone(&self.transport),
                clock: Arc::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Duration::ZERO
                }),
                sleeper: Arc::new(NoopSleeper),
                flush_interval: Duration::from_secs(5),
                tasks: self.tasks.clone(),
            },
        )
    }
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .expect("the capture lock is not poisoned")
            .clone();
        String::from_utf8(bytes).expect("the captured output is UTF-8")
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("the capture lock is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_debug() -> (CapturedLogs, DefaultGuard) {
    let captured = CapturedLogs::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

#[test]
fn credentials_read_the_key_and_region() {
    let params = map(&[("newrelic_api_key", "team-key"), ("newrelic_region", "eu")]);
    let credentials = newrelic_credentials(&params);
    assert_eq!(credentials.newrelic_api_key.as_deref(), Some("team-key"));
    assert_eq!(credentials.newrelic_region.as_deref(), Some("eu"));
}

#[test]
fn credentials_leave_absent_parameters_as_none() {
    let credentials = newrelic_credentials(&map(&[]));
    assert!(credentials.newrelic_api_key.is_none());
    assert!(credentials.newrelic_region.is_none());
}

#[rstest]
#[case::present_key(&[("newrelic_api_key", "team-key")], true)]
#[case::empty_key(&[("newrelic_api_key", "")], true)]
#[case::region_alone(&[("newrelic_region", "eu")], false)]
#[case::no_parameters(&[], false)]
fn credentials_passed_follows_the_api_key_presence(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: bool,
) {
    assert_eq!(newrelic_credentials_passed(&map(pairs)), expected);
}

#[tokio::test]
async fn the_same_credentials_return_the_same_cached_logger() {
    let fixture = fixture();
    let params = map(&[("newrelic_api_key", "team-key"), ("newrelic_region", "eu")]);
    let first = fixture.get(&params).expect("the first logger");
    let second = fixture.get(&params).expect("the cached logger");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(fixture.builds.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_different_credential_set_builds_a_second_logger() {
    let fixture = fixture();
    let first = fixture
        .get(&map(&[("newrelic_api_key", "team-a")]))
        .expect("the first team logger");
    let second = fixture
        .get(&map(&[("newrelic_api_key", "team-b")]))
        .expect("the second team logger");
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(fixture.builds.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn an_absent_region_and_an_empty_one_are_different_cache_keys() {
    let fixture = fixture();
    let absent = fixture
        .get(&map(&[("newrelic_api_key", "team-key")]))
        .expect("the absent-region logger");
    let empty = fixture
        .get(&map(&[
            ("newrelic_api_key", "team-key"),
            ("newrelic_region", ""),
        ]))
        .expect("the empty-region logger");
    assert!(!Arc::ptr_eq(&absent, &empty));
    assert_eq!(fixture.builds.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn an_absent_key_is_refused_as_the_empty_string() {
    let fixture = fixture();
    let result = fixture.get(&map(&[("newrelic_region", "eu")]));
    assert!(matches!(
        result,
        Err(NewRelicTeamHandlerError::Build(
            NewRelicMetricsError::MissingApiKey
        ))
    ));
}

#[tokio::test]
async fn an_empty_key_is_refused() {
    let fixture = fixture();
    let result = fixture.get(&map(&[("newrelic_api_key", "")]));
    assert!(matches!(
        result,
        Err(NewRelicTeamHandlerError::Build(
            NewRelicMetricsError::MissingApiKey
        ))
    ));
}

#[tokio::test]
async fn the_metrics_logger_delivers_with_the_given_key_and_region() {
    let fixture = fixture();
    let params = map(&[("newrelic_api_key", "team-key"), ("newrelic_region", "eu")]);
    let logger = fixture.get(&params).expect("the team metrics logger");
    logger.record_event(&payload()).await;
    assert!(logger.stop());
    fixture.tasks.close();
    timeout(Duration::from_secs(5), fixture.tasks.wait())
        .await
        .expect("the flush tasks end");
    let requests = fixture
        .transport
        .requests
        .lock()
        .expect("the request lock is not poisoned");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, EU_METRIC_ENDPOINT);
    assert_eq!(
        requests[0]
            .1
            .get("api-key")
            .expect("the team ingest key header")
            .to_str()
            .expect("a visible key"),
        "team-key"
    );
}

#[tokio::test]
async fn a_new_logger_logs_the_team_scope_without_the_credentials() {
    let (captured, _guard) = capture_debug();
    let fixture = fixture();
    let logger = fixture
        .get(&map(&[("newrelic_api_key", "team-secret-key")]))
        .expect("the team metrics logger");
    let text = captured.text();
    assert!(text.contains(
        "New Relic: Created and cached new NewRelicMetricsLogger for team-scoped credentials"
    ));
    assert!(!text.contains("team-secret-key"));
    assert_eq!(logger.callback_name(), "newrelic");
}
