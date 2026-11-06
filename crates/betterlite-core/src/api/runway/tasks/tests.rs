#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::Instant;
use url::Url;

use super::TaskKind;
use super::error::RunwayTaskError;
use super::poll::{MonotonicClock, PollClock, TaskPoller, TaskTransport};
use super::record::{RunwayTask, TaskOutput, TaskOutputs, decode_runway_task};
use super::settings::RunwaySettings;

struct Script {
    answers: Mutex<VecDeque<(u16, &'static str)>>,
    urls: Mutex<Vec<String>>,
}

impl Script {
    fn new(answers: &[(u16, &'static str)]) -> Self {
        Self {
            answers: Mutex::new(answers.iter().copied().collect()),
            urls: Mutex::new(Vec::new()),
        }
    }
}

impl TaskTransport for Script {
    fn get(
        &self,
        url: Url,
        _headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.urls.lock().unwrap().push(url.to_string());
        let (status, body) = self.answers.lock().unwrap().pop_front().unwrap();
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        }))
    }
}

struct Clock {
    millis: AtomicU64,
    step: u64,
    pauses: Mutex<Vec<Duration>>,
}

impl Clock {
    fn new(step: u64) -> Self {
        Self {
            millis: AtomicU64::new(0),
            step,
            pauses: Mutex::new(Vec::new()),
        }
    }
}

impl PollClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.step, Ordering::SeqCst))
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.pauses.lock().unwrap().push(duration);
        ready(())
    }
}

fn settings(pairs: &[(&str, &str)]) -> Result<RunwaySettings, RunwayTaskError> {
    let lookup: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RunwaySettings::load(&lookup)
}

async fn poll_with(
    answers: &[(u16, &'static str)],
    kind: TaskKind,
    timeout_seconds: u64,
    step: u64,
) -> (Result<RunwayTask, RunwayTaskError>, Script, Clock) {
    let transport = Script::new(answers);
    let clock = Clock::new(step);
    let poller = TaskPoller {
        transport: &transport,
        clock: &clock,
        kind,
        timeout_seconds,
    };
    let outcome = poller
        .poll("https://runway.test/", "task-1", &HeaderMap::new())
        .await;
    (outcome, transport, clock)
}

#[test]
fn runway_task_settings() {
    let defaults = settings(&[]).unwrap();
    assert_eq!(defaults.api_base, "https://api.dev.runwayml.com");
    assert_eq!(defaults.api_version, "2024-11-06");
    assert_eq!(defaults.polling_timeout_seconds, 600);
    assert!(defaults.api_secret.is_none() && defaults.api_key.is_none());
    let empty = settings(&[("RUNWAYML_API_BASE", ""), ("RUNWAYML_POLLING_TIMEOUT", "")]).unwrap();
    assert_eq!(empty.api_base, "https://api.dev.runwayml.com");
    assert_eq!(empty.polling_timeout_seconds, 600);
    let set = settings(&[
        ("RUNWAYML_API_BASE", "https://other.test"),
        ("RUNWAYML_DEFAULT_API_VERSION", "2025-01-01"),
        ("RUNWAYML_POLLING_TIMEOUT", "30"),
    ])
    .unwrap();
    assert_eq!(set.api_base, "https://other.test");
    assert_eq!(set.api_version, "2025-01-01");
    assert_eq!(set.polling_timeout_seconds, 30);
    let secrets = settings(&[
        ("RUNWAYML_API_SECRET", "secret-1"),
        ("RUNWAYML_API_KEY", "key-1"),
    ])
    .unwrap();
    assert_eq!(secrets.api_secret.unwrap().expose_secret(), "secret-1");
    assert_eq!(secrets.api_key.unwrap().expose_secret(), "key-1");
    let empty_secrets = settings(&[("RUNWAYML_API_SECRET", ""), ("RUNWAYML_API_KEY", "")]).unwrap();
    assert!(empty_secrets.api_secret.is_none() && empty_secrets.api_key.is_none());
    let bad = settings(&[("RUNWAYML_POLLING_TIMEOUT", "soon")]).unwrap_err();
    assert!(bad.to_string().contains("RUNWAYML_POLLING_TIMEOUT"));
}

#[rstest]
#[case::explicit(Some("explicit"), Some("secret"), Some("key"), "Bearer explicit")]
#[case::empty_explicit(Some(""), Some("secret"), Some("key"), "Bearer secret")]
#[case::secret(None, Some("secret"), Some("key"), "Bearer secret")]
#[case::empty_secret(None, Some(""), Some("key"), "Bearer key")]
#[case::key(None, None, Some("key"), "Bearer key")]
fn header_fallback(
    #[case] explicit: Option<&str>,
    #[case] secret: Option<&str>,
    #[case] key: Option<&str>,
    #[case] expected: &str,
) {
    let loaded = RunwaySettings {
        api_secret: secret.map(SecretString::from),
        api_key: key.map(SecretString::from),
        ..settings(&[]).unwrap()
    };
    let mut caller = HeaderMap::new();
    caller.insert("x-trace", HeaderValue::from_static("t-1"));
    caller.insert(AUTHORIZATION, HeaderValue::from_static("Bearer old"));
    let credential = explicit.map(SecretString::from);
    let headers = loaded
        .authenticate(TaskKind::Speech, &caller, credential.as_ref())
        .unwrap();
    assert_eq!(headers[AUTHORIZATION], expected);
    assert!(headers[AUTHORIZATION].is_sensitive());
    assert_eq!(headers["x-runway-version"], "2024-11-06");
    assert_eq!(headers["x-trace"], "t-1");
    assert_eq!(headers[CONTENT_TYPE], "application/json");
    let image = loaded
        .authenticate(TaskKind::Image, &caller, credential.as_ref())
        .unwrap();
    assert!(!image.contains_key(CONTENT_TYPE));
}

#[test]
fn header_fallback_without_credentials() {
    let error = settings(&[])
        .unwrap()
        .authenticate(TaskKind::Image, &HeaderMap::new(), None)
        .unwrap_err();
    assert!(matches!(error, RunwayTaskError::MissingCredentials));
}

#[rstest]
#[case::image(TaskKind::Image, None, "https://api.dev.runwayml.com/v1/text_to_image")]
#[case::speech(
    TaskKind::Speech,
    None,
    "https://api.dev.runwayml.com/v1/text_to_speech"
)]
#[case::explicit(
    TaskKind::Image,
    Some("https://x.test/"),
    "https://x.test/v1/text_to_image"
)]
#[case::empty_explicit(
    TaskKind::Speech,
    Some(""),
    "https://api.dev.runwayml.com/v1/text_to_speech"
)]
fn endpoint_suffix(#[case] kind: TaskKind, #[case] base: Option<&str>, #[case] expected: &str) {
    assert_eq!(settings(&[]).unwrap().endpoint(kind, base), expected);
}

#[tokio::test]
async fn throttled_then_success() {
    let (outcome, transport, clock) = poll_with(
        &[
            (200, r#"{"status":"throttled"}"#),
            (200, r#"{"status":"Pending"}"#),
            (
                200,
                r#"{"id":"task-1","status":"SUCCEEDED","output":["https://o.test/a.png"]}"#,
            ),
        ],
        TaskKind::Image,
        600,
        0,
    )
    .await;
    let task = outcome.unwrap();
    assert_eq!(task.id.as_deref(), Some("task-1"));
    assert_eq!(
        task.output,
        Some(TaskOutputs::Members(vec![TaskOutput::Url(
            "https://o.test/a.png".to_owned()
        )]))
    );
    let urls = transport.urls.lock().unwrap();
    assert_eq!(urls.len(), 3);
    assert_eq!(urls[0], "https://runway.test/v1/tasks/task-1");
    assert_eq!(*clock.pauses.lock().unwrap(), [Duration::from_secs(2); 2]);
}

#[rstest]
#[case::at_the_limit(1000, 1)]
#[case::past_the_limit(1001, 0)]
#[tokio::test]
async fn deadline_equality(#[case] step_millis: u64, #[case] gets: usize) {
    let running = (200, r#"{"status":"RUNNING"}"#);
    let (outcome, transport, _) = poll_with(&[running; 8], TaskKind::Image, 1, step_millis).await;
    assert!(matches!(
        outcome.unwrap_err(),
        RunwayTaskError::TimedOut { .. }
    ));
    assert_eq!(transport.urls.lock().unwrap().len(), gets);
}

#[rstest]
#[case::image_failure(
    TaskKind::Image,
    r#"{"status":"FAILED","failure":"bad prompt","failureCode":"SAFETY"}"#,
    "RunwayML image generation failed: bad prompt (code: SAFETY)"
)]
#[case::image_unknown(
    TaskKind::Image,
    r#"{"status":"failed"}"#,
    "RunwayML image generation failed: Unknown error (code: unknown)"
)]
#[case::speech_failure(
    TaskKind::Speech,
    r#"{"status":"FAILED","failure":"no voice","failureCode":"VOICE"}"#,
    "RunwayML TTS failed: no voice (code: VOICE)"
)]
#[case::speech_unknown(
    TaskKind::Speech,
    r#"{"status":"FAILED"}"#,
    "RunwayML TTS failed: Unknown error (code: unknown)"
)]
#[case::image_cancelled(
    TaskKind::Image,
    r#"{"status":"CANCELLED"}"#,
    "RunwayML image generation was cancelled"
)]
#[case::speech_cancelled(
    TaskKind::Speech,
    r#"{"status":"cancelled"}"#,
    "RunwayML TTS was cancelled"
)]
#[case::unknown_status(
    TaskKind::Image,
    r#"{"status":"weird"}"#,
    "Unknown RunwayML task status: WEIRD"
)]
#[case::missing_status(TaskKind::Image, "{}", "Unknown RunwayML task status: ")]
#[tokio::test]
async fn failed_messages(
    #[case] kind: TaskKind,
    #[case] body: &'static str,
    #[case] message: &str,
) {
    let (outcome, transport, _) = poll_with(&[(200, body)], kind, 600, 0).await;
    assert_eq!(outcome.unwrap_err().to_string(), message);
    assert_eq!(transport.urls.lock().unwrap().len(), 1);
}

#[rstest]
#[case::image(TaskKind::Image, "RunwayML task polling timed out after 0 seconds")]
#[case::speech(
    TaskKind::Speech,
    "RunwayML TTS task polling timed out after 0 seconds"
)]
#[tokio::test]
async fn timeout_messages(#[case] kind: TaskKind, #[case] message: &str) {
    let (outcome, transport, _) = poll_with(&[], kind, 0, 1).await;
    assert_eq!(outcome.unwrap_err().to_string(), message);
    assert!(transport.urls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn http_error_before_json() {
    let (outcome, ..) = poll_with(&[(502, "not json")], TaskKind::Image, 600, 0).await;
    assert!(matches!(
        outcome.unwrap_err(),
        RunwayTaskError::HttpStatus { status: 502 }
    ));
}

#[tokio::test]
async fn task_id_is_one_path_segment() {
    let transport = Script::new(&[(200, r#"{"status":"SUCCEEDED"}"#)]);
    let clock = Clock::new(0);
    let poller = TaskPoller {
        transport: &transport,
        clock: &clock,
        kind: TaskKind::Image,
        timeout_seconds: 600,
    };
    poller
        .poll("https://runway.test", "a/b c?d", &HeaderMap::new())
        .await
        .unwrap();
    let urls = transport.urls.lock().unwrap();
    assert_eq!(urls[0], "https://runway.test/v1/tasks/a%2Fb%20c%3Fd");
}

#[rstest]
#[case::empty("", "task_id is required")]
#[case::dot(".", "task_id cannot be a dot path segment")]
#[case::dot_dot("..", "task_id cannot be a dot path segment")]
#[tokio::test]
async fn rejects_an_unusable_task_id(#[case] task_id: &str, #[case] message: &str) {
    let transport = Script::new(&[]);
    let clock = Clock::new(0);
    let poller = TaskPoller {
        transport: &transport,
        clock: &clock,
        kind: TaskKind::Image,
        timeout_seconds: 600,
    };
    let error = poller
        .poll("https://runway.test", task_id, &HeaderMap::new())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
    assert!(matches!(error, RunwayTaskError::TaskId(_)));
    assert!(transport.urls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn monotonic_clock_passes_real_time() {
    let clock = MonotonicClock {
        origin: Instant::now(),
    };
    let started = clock.now();
    clock.pause(Duration::from_millis(100)).await;
    assert!(clock.now() >= started + Duration::from_millis(20));
}

#[test]
fn task_bytes_at_the_limit_decode() {
    let mut body = br#"{"status":"SUCCEEDED","pad":""#.to_vec();
    body.resize(0x0100_0000 - 2, b'x');
    body.extend_from_slice(b"\"}");
    assert_eq!(body.len(), 0x0100_0000);
    let task = decode_runway_task(&body).unwrap();
    assert_eq!(task.state(), super::record::TaskState::Succeeded);
}

#[test]
fn oversized_task_is_refused() {
    let body = vec![b' '; 0x0100_0001];
    assert!(matches!(
        decode_runway_task(&body),
        Err(RunwayTaskError::TooLarge { .. })
    ));
}

#[test]
fn mixed_outputs_keep_every_member() {
    let task = decode_runway_task(
        br#"{"status":"SUCCEEDED","output":["u",{"url":"v"},7,{"n":1}],"extra":true}"#,
    )
    .unwrap();
    let Some(TaskOutputs::Members(members)) = task.output else {
        panic!("output should be a list");
    };
    assert_eq!(members[0], TaskOutput::Url("u".to_owned()));
    assert_eq!(
        members[1],
        TaskOutput::Fields(BTreeMap::from([("url".to_owned(), "v".to_owned())]))
    );
    assert_eq!(members[2], TaskOutput::Unsupported("7".to_owned()));
    assert_eq!(members[3], TaskOutput::Unsupported(r#"{"n":1}"#.to_owned()));
}

#[rstest]
fn replays_the_runway_task_corpus(#[files("fuzz/corpus/runway_task/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(task) = decode_runway_task(&bytes) {
        assert_eq!(str::from_utf8(&bytes).map(str::len), Ok(bytes.len()));
        drop(task.state());
    }
}
