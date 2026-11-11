#![cfg(test)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;
use url::Url;

use crate::callback_logger::CallbackLogger as _;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::{Clock, Credentials, StaticCredentials};
use crate::s3_transport::transport::{S3HttpClient, S3Request, S3Response, S3Transport};
use crate::s3_transport::url_construction::S3Endpoint;
use crate::sqs_logging::tests::support::{environment, event, payload};

use super::S3Logger;
use super::settings::{self, Settings};

const ACCESS_KEY_ID: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_ACCESS_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const FIXED_TIME_SECONDS: u64 = 1_767_323_045;

fn fixed_time() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_TIME_SECONDS))
        .expect("the fixed time is representable")
}

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[derive(Clone, Copy, Debug)]
enum Answer {
    Status(u16),
    Unreachable,
}

#[derive(Debug)]
struct FakeS3Client {
    requests: Arc<Mutex<Vec<S3Request>>>,
    answer: Answer,
}

impl S3HttpClient for FakeS3Client {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let answer = self.answer;
        Box::pin(async move {
            match answer {
                Answer::Status(status) => Ok(S3Response {
                    status,
                    body: Vec::new(),
                }),
                Answer::Unreachable => Err(S3TransportError::MissingBucket),
            }
        })
    }
}

fn params(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings_from(pairs: &[(&str, &str)]) -> Settings {
    settings::resolve(&params(pairs)).expect("the S3 settings resolve")
}

fn transport(
    settings: &Settings,
    requests: &Arc<Mutex<Vec<S3Request>>>,
    answer: Answer,
) -> S3Transport {
    S3Transport {
        client: Arc::new(FakeS3Client {
            requests: Arc::clone(requests),
            answer,
        }),
        endpoint: S3Endpoint {
            bucket: "logs".to_owned(),
            region: "us-east-1".to_owned(),
            endpoint_url: None,
            virtual_hosted_style: false,
        },
        credentials: Arc::new(StaticCredentials {
            credentials: Credentials {
                access_key_id: SecretString::from(ACCESS_KEY_ID),
                secret_access_key: SecretString::from(SECRET_ACCESS_KEY),
                session_token: None,
            },
        }),
        encryption: settings.encryption.clone(),
        drop_on_terminal_error: true,
        throttle: None,
        clock: Arc::new(FixedClock(fixed_time())),
    }
}

struct Harness {
    logger: S3Logger,
    requests: Arc<Mutex<Vec<S3Request>>>,
    tasks: TaskTracker,
}

fn harness(pairs: &[(&str, &str)], variables: &[(&str, &str)]) -> Harness {
    harness_answering(pairs, variables, Answer::Status(200))
}

fn harness_answering(
    pairs: &[(&str, &str)],
    variables: &[(&str, &str)],
    answer: Answer,
) -> Harness {
    let settings = settings_from(pairs);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = transport(&settings, &requests, answer);
    let tasks = TaskTracker::new();
    let logger = S3Logger::new(
        settings,
        Some(transport),
        Arc::new(environment(variables)),
        tasks.clone(),
    );
    Harness {
        logger,
        requests,
        tasks,
    }
}

fn recorded(requests: &Arc<Mutex<Vec<S3Request>>>) -> Vec<S3Request> {
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn header<'request>(request: &'request S3Request, wanted: &str) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case(wanted))
        .map(|(_name, value)| value.as_str())
}

fn path(request: &S3Request) -> String {
    Url::parse(&request.url)
        .expect("the object URL parses")
        .path()
        .to_owned()
}

fn body(request: &S3Request) -> Json {
    let text = String::from_utf8(request.body.clone()).expect("the body is UTF-8");
    serde_json::from_str(&text).expect("the body is JSON")
}

fn assert_upload_headers(request: &S3Request) {
    assert_eq!(header(request, "content-type"), Some("application/json"));
    assert_eq!(header(request, "content-language"), Some("en"));
    assert_eq!(
        header(request, "content-disposition"),
        Some("inline; filename=\"time-2009-02-13T23-31-30-000000_sqs-health-check.json\"")
    );
    assert_eq!(
        header(request, "cache-control"),
        Some("private, immutable, max-age=31536000, s-maxage=0")
    );
    assert!(header(request, "authorization").is_some());
    assert!(header(request, "x-amz-content-sha256").is_some());
}

#[tokio::test]
async fn callback_logger_uploads_the_standard_payload() {
    let harness = harness(
        &[
            ("s3_bucket_name", "logs"),
            ("s3_region_name", "us-east-1"),
            ("s3_path", "team-logs/"),
            ("s3_aws_access_key_id", ACCESS_KEY_ID),
            ("s3_aws_secret_access_key", SECRET_ACCESS_KEY),
        ],
        &[],
    );
    harness
        .logger
        .async_log_success_event(event(payload()))
        .await
        .expect("the event is accepted");
    let requests = recorded(&harness.requests);
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        path(request),
        "/team-logs/2009-02-13/time-23-31-30-000000_sqs-health-check.json"
    );
    assert_upload_headers(request);
    let Json::Object(fields) = body(request) else {
        panic!("the uploaded body is an object");
    };
    assert_eq!(
        fields.get("id"),
        Some(&Json::Text("sqs-health-check".to_owned()))
    );
}

#[tokio::test]
async fn callback_logger_skips_a_missing_payload() {
    let harness = harness(&[("s3_bucket_name", "logs")], &[]);
    harness.logger.uploads.upload(None).await;
    assert!(recorded(&harness.requests).is_empty());
}

#[tokio::test]
async fn callback_logger_skips_when_no_bucket_is_configured() {
    let harness = harness(&[], &[]);
    harness.logger.uploads.upload(Some(&payload())).await;
    assert!(recorded(&harness.requests).is_empty());
}

#[tokio::test]
async fn callback_logger_swallows_a_rejected_upload() {
    let harness = harness_answering(&[("s3_bucket_name", "logs")], &[], Answer::Status(400));
    harness
        .logger
        .async_log_success_event(event(payload()))
        .await
        .expect("the callback never propagates a failure");
    assert_eq!(recorded(&harness.requests).len(), 1);
}

#[tokio::test]
async fn callback_logger_swallows_an_unreachable_upload() {
    let harness = harness_answering(&[("s3_bucket_name", "logs")], &[], Answer::Unreachable);
    harness
        .logger
        .async_log_success_event(event(payload()))
        .await
        .expect("the callback never propagates a failure");
    assert_eq!(recorded(&harness.requests).len(), 1);
}

#[tokio::test]
async fn callback_logger_sends_the_sync_event_through_the_task_tracker() {
    let harness = harness(&[("s3_bucket_name", "logs")], &[]);
    let event = event(payload());
    harness
        .logger
        .log_success_event(&event)
        .expect("the event is accepted");
    harness.tasks.close();
    harness.tasks.wait().await;
    assert_eq!(recorded(&harness.requests).len(), 1);
}

#[rstest]
#[case::preview_and_prefix("True", "True", "team", "/team/")]
#[case::preview_off("False", "True", "team", "/")]
#[case::prefix_off("True", "False", "team", "/")]
#[case::no_alias("True", "True", "none", "/")]
#[case::empty_alias("True", "True", "", "//")]
#[tokio::test]
async fn callback_logger_applies_the_team_prefix_only_with_preview_features(
    #[case] preview: &str,
    #[case] use_prefix: &str,
    #[case] alias: &str,
    #[case] expected_prefix: &str,
) {
    let harness = harness(
        &[
            ("s3_bucket_name", "logs"),
            ("enable_preview_features", preview),
            ("s3_use_team_prefix", use_prefix),
        ],
        &[],
    );
    let mut standard = payload();
    standard.metadata.user_api_key_team_alias = match alias {
        "none" => None,
        present => Some(present.to_owned()),
    };
    harness
        .logger
        .async_log_success_event(event(standard))
        .await
        .expect("the event is accepted");
    let requests = recorded(&harness.requests);
    let object_path = path(&requests[0]);
    assert!(
        object_path.starts_with(expected_prefix),
        "{object_path} starts with {expected_prefix}"
    );
    assert!(object_path.ends_with("/2009-02-13/time-23-31-30-000000_sqs-health-check.json"));
}

#[rstest]
#[case::configured_on("True", "", true)]
#[case::configured_off("False", "", false)]
#[case::environment_on("", "true", true)]
#[case::unset("", "", false)]
#[tokio::test]
async fn callback_logger_applies_the_prompts_only_projection(
    #[case] configured: &str,
    #[case] environment_prompts_only: &str,
    #[case] response_nulled: bool,
) {
    let mut pairs = vec![("s3_bucket_name", "logs")];
    if !configured.is_empty() {
        pairs.push(("s3_log_prompts_only", configured));
    }
    let mut variables: Vec<(&str, &str)> = Vec::new();
    if !environment_prompts_only.is_empty() {
        variables.push(("S3_LOG_PROMPTS_ONLY", environment_prompts_only));
    }
    let harness = harness(&pairs, &variables);
    let mut standard = payload();
    standard.response = Some(Json::Text("resp".to_owned()));
    harness
        .logger
        .async_log_success_event(event(standard))
        .await
        .expect("the event is accepted");
    let requests = recorded(&harness.requests);
    let Json::Object(fields) = body(&requests[0]) else {
        panic!("the uploaded body is an object");
    };
    let expected = if response_nulled {
        Json::Null
    } else {
        Json::Text("resp".to_owned())
    };
    assert_eq!(fields.get("response"), Some(&expected));
}

#[rstest]
#[case::aes("AES256", "", "AES256", "")]
#[case::kms_key_only("", "key-1", "aws:kms", "key-1")]
#[case::aes_with_key("AES256", "key-1", "AES256", "")]
#[tokio::test]
async fn callback_logger_attaches_the_sse_headers(
    #[case] algorithm: &str,
    #[case] kms_key: &str,
    #[case] expected_algorithm: &str,
    #[case] expected_key: &str,
) {
    let mut pairs = vec![("s3_bucket_name", "logs")];
    if !algorithm.is_empty() {
        pairs.push(("s3_server_side_encryption", algorithm));
    }
    if !kms_key.is_empty() {
        pairs.push(("s3_sse_kms_key_id", kms_key));
    }
    let harness = harness(&pairs, &[]);
    harness
        .logger
        .async_log_success_event(event(payload()))
        .await
        .expect("the event is accepted");
    let requests = recorded(&harness.requests);
    let algorithm_header = if expected_algorithm.is_empty() {
        None
    } else {
        Some(expected_algorithm)
    };
    let key_header = if expected_key.is_empty() {
        None
    } else {
        Some(expected_key)
    };
    assert_eq!(
        header(&requests[0], "x-amz-server-side-encryption"),
        algorithm_header
    );
    assert_eq!(
        header(&requests[0], "x-amz-server-side-encryption-aws-kms-key-id"),
        key_header
    );
}
