use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use chrono::DateTime;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::finops::focus::destinations::focus_gcs_destination::{FocusGcsDestination, GcsSettings};
use crate::finops::focus::destinations::focus_s3_destination::{
    FocusS3Destination, S3Connection, S3Settings,
};
use crate::focus_records::destinations::FocusTimeWindow;
use crate::gcs_client::gcs_storage_auth::GcsBucketBase;
use crate::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
use crate::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use crate::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use crate::gcs_mock::request::{GcsRequest, GcsWriteRequest};
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::{S3HttpClient, S3Request, S3Response};

pub(super) const BUCKET: &str = "focus-exports";
pub(super) const PREFIX: &str = "exports";
pub(super) const FILENAME: &str = "usage_20260102T030405Z_20260102T040405Z.csv";
pub(super) const CONTENT: &[u8] = b"ChargeCategory,Usage\n";
pub(super) const TOKEN: &str = "test-token";
pub(super) const SERVICE_ACCOUNT: &str = "/secrets/focus-service-account.json";
pub(super) const ENVIRONMENT_SERVICE_ACCOUNT: &str = "/env/service-account.json";
pub(super) const START_EPOCH: i64 = 1_767_323_045;

pub(super) fn window(start_epoch: i64, frequency: &str) -> FocusTimeWindow {
    FocusTimeWindow {
        start_time: DateTime::from_timestamp(start_epoch, 0).unwrap(),
        end_time: DateTime::from_timestamp(start_epoch + 3_600, 0).unwrap(),
        frequency: frequency.to_owned(),
    }
}

pub(super) fn s3_settings() -> S3Settings {
    S3Settings {
        bucket_name: Some(BUCKET.to_owned()),
        region_name: Some("us-east-1".to_owned()),
        endpoint_url: Some("https://s3.example.test".to_owned()),
        aws_access_key_id: Some(SecretString::from("access-key")),
        aws_secret_access_key: Some(SecretString::from("secret-key")),
        aws_session_token: Some(SecretString::from("session-token")),
    }
}

pub(super) fn s3_bucket_only_settings() -> S3Settings {
    S3Settings {
        bucket_name: Some(BUCKET.to_owned()),
        region_name: None,
        endpoint_url: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
    }
}

pub(super) fn gcs_settings() -> GcsSettings {
    GcsSettings {
        bucket_name: Some(BUCKET.to_owned()),
        service_account_json: Some(SecretString::from(SERVICE_ACCOUNT)),
    }
}

pub(super) fn gcs_bucket_only_settings() -> GcsSettings {
    GcsSettings {
        bucket_name: Some(BUCKET.to_owned()),
        service_account_json: None,
    }
}

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8(bytes).expect("captured logs are UTF-8")
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Captured {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

#[derive(Debug, thiserror::Error)]
#[error("the network is down")]
pub(super) struct NetworkDown;

#[derive(Clone, Debug)]
struct Answer {
    status: u16,
    body: Vec<u8>,
    failing: bool,
}

impl Answer {
    fn answering(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            body: body.to_vec(),
            failing: false,
        }
    }

    fn down() -> Self {
        Self {
            status: 200,
            body: Vec::new(),
            failing: true,
        }
    }
}

#[derive(Debug)]
pub(super) struct FakeS3Http {
    sent: Mutex<Vec<S3Request>>,
    answer: Answer,
}

impl FakeS3Http {
    pub(super) fn answering(status: u16, body: &[u8]) -> Arc<Self> {
        Arc::new(Self {
            sent: Mutex::default(),
            answer: Answer::answering(status, body),
        })
    }

    pub(super) fn failing() -> Arc<Self> {
        Arc::new(Self {
            sent: Mutex::default(),
            answer: Answer::down(),
        })
    }

    pub(super) fn sent(&self) -> Vec<S3Request> {
        self.sent.lock().unwrap().clone()
    }
}

impl S3HttpClient for FakeS3Http {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        self.sent.lock().unwrap().push(request);
        let outcome = if self.answer.failing {
            Err(S3TransportError::MissingBucket)
        } else {
            Ok(S3Response {
                status: self.answer.status,
                body: self.answer.body.clone(),
            })
        };
        Box::pin(async move { outcome })
    }
}

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(u64::try_from(START_EPOCH).unwrap())
    }
}

pub(super) fn header<'request>(request: &'request S3Request, name: &str) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

pub(super) fn s3_connection(variables: &[(&str, &str)], http: &Arc<FakeS3Http>) -> S3Connection {
    let shared = Arc::clone(http);
    let client: Arc<dyn S3HttpClient> = shared;
    S3Connection {
        http: client,
        environment: Arc::new(environment(variables)),
        clock: Arc::new(FixedClock),
    }
}

pub(super) fn s3_destination_with(
    prefix: &str,
    settings: S3Settings,
    variables: &[(&str, &str)],
    http: &Arc<FakeS3Http>,
) -> FocusS3Destination {
    let connection = s3_connection(variables, http);
    FocusS3Destination::new(connection, prefix, settings).unwrap()
}

pub(super) fn s3_destination(
    prefix: &str,
    settings: S3Settings,
) -> (FocusS3Destination, Arc<FakeS3Http>) {
    let http = FakeS3Http::answering(200, b"");
    let destination = s3_destination_with(prefix, settings, &[], &http);
    (destination, http)
}

#[derive(Clone, Debug, Default)]
pub(super) struct FakeAuth {
    failing: bool,
    ensured: Arc<Mutex<Vec<Option<String>>>>,
}

impl FakeAuth {
    pub(super) fn failing() -> Self {
        Self {
            failing: true,
            ensured: Arc::default(),
        }
    }

    // the service account path each token request carried
    pub(super) fn ensured(&self) -> Vec<Option<String>> {
        self.ensured.lock().unwrap().clone()
    }

    fn token(&self, request: &EnsureAccessToken) -> Result<AccessToken, NetworkDown> {
        self.ensured.lock().unwrap().push(
            request
                .credentials
                .as_ref()
                .map(|value| value.expose_secret().to_owned()),
        );
        if self.failing {
            return Err(NetworkDown);
        }
        Ok(AccessToken {
            token: SecretString::from(TOKEN),
            project_id: "test-project".to_owned(),
        })
    }
}

#[async_trait]
impl VertexAuth for FakeAuth {
    type Error = NetworkDown;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, NetworkDown> {
        self.token(&request)
    }

    fn ensure_access_token(&self, request: EnsureAccessToken) -> Result<AccessToken, NetworkDown> {
        self.token(&request)
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, NetworkDown> {
        Ok(TokenAndUrl {
            token: request.auth_header.unwrap(),
            url: String::new(),
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct FakeGcsHttp {
    sent: Arc<Mutex<Vec<GcsRequest>>>,
    answer: Answer,
}

impl FakeGcsHttp {
    pub(super) fn answering(status: u16, body: &[u8]) -> Self {
        Self {
            sent: Arc::default(),
            answer: Answer::answering(status, body),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            sent: Arc::default(),
            answer: Answer::down(),
        }
    }

    pub(super) fn posts(&self) -> Vec<GcsWriteRequest> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|request| match request {
                GcsRequest::Post(post) => post.clone(),
                other => panic!("the destination sent a request that is not a POST: {other:?}"),
            })
            .collect()
    }
}

#[async_trait]
impl GcsHttpClient for FakeGcsHttp {
    type Error = NetworkDown;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, NetworkDown> {
        let url = request.url().to_owned();
        self.sent.lock().unwrap().push(request);
        if self.answer.failing {
            return Err(NetworkDown);
        }
        Ok(GcsHttpResponse {
            status: self.answer.status,
            headers: HeaderMap::new(),
            body: self.answer.body.clone(),
            url,
            elapsed: Duration::ZERO,
        })
    }
}

pub(super) fn gcs_base(
    auth: FakeAuth,
    http: FakeGcsHttp,
    variables: &[(&str, &str)],
) -> GcsBucketBase<FakeAuth, FakeGcsHttp> {
    let source = Arc::new(environment(variables));
    initialize(source, Some(BUCKET.to_owned()), auth, http).unwrap()
}

pub(super) fn gcs_destination(
    prefix: &str,
    settings: GcsSettings,
) -> (FocusGcsDestination<FakeAuth, FakeGcsHttp>, FakeGcsHttp) {
    let http = FakeGcsHttp::answering(200, b"");
    let base = gcs_base(FakeAuth::default(), http.clone(), &[]);
    let destination = FocusGcsDestination::new(base, prefix, settings).unwrap();
    (destination, http)
}
