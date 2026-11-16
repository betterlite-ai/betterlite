use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::focus::destinations::factory::FocusDestinationFactory;
use crate::finops::focus::destinations::factory::resolve::DestinationOverrides;
use crate::finops::vantage::sender::{CsvSender, MultipartPost};
use crate::focus_records::destinations::FocusTimeWindow;
use crate::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use crate::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
use crate::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use crate::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use crate::gcs_mock::request::GcsRequest;
use crate::mavvrik_delivery::transport::{Method, Transport, TransportRequest, TransportResponse};
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::{S3HttpClient, S3Request, S3Response};

pub(super) const BUCKET: &str = "focus-exports";
pub(super) const PREFIX: &str = "exports";
pub(super) const ACCESS_KEY: &str = "focus-access-key";
pub(super) const SECRET_KEY: &str = "focus-secret-key";
pub(super) const VANTAGE_KEY: &str = "vantage-key";
pub(super) const VANTAGE_TOKEN: &str = "vantage-token";
pub(super) const SERVICE_ACCOUNT: &str = "/secrets/focus-service-account.json";
pub(super) const MAVVRIK_ENDPOINT: &str = "https://api.mavvrik.dev/tenant";
pub(super) const MAVVRIK_CONNECTION: &str = "connection-1";
pub(super) const MAVVRIK_KEY: &str = "mavvrik-key";
pub(super) const SIGNED_URL: &str =
    "https://storage.googleapis.com/bucket/object?X-Goog-Signature=abc";
pub(super) const SESSION_URI: &str = "https://storage.googleapis.com/upload/session-1";

pub(super) type TestFactory =
    FocusDestinationFactory<FakeGcsAuth, FakeGcsHttp, FakeCsvSender, FakeMavvrikTransport>;

pub(super) fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn factory(variables: &[(&str, &str)]) -> TestFactory {
    factory_with_gcs_status(variables, 200)
}

pub(super) fn destination_overrides() -> DestinationOverrides {
    DestinationOverrides {
        bucket_name: None,
        region_name: None,
        endpoint_url: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        api_key: None,
        integration_token: None,
        base_url: None,
        service_account_json: None,
        api_endpoint: None,
        connection_id: None,
    }
}

pub(super) fn factory_with_gcs_status(variables: &[(&str, &str)], gcs_status: u16) -> TestFactory {
    build_factory(variables, gcs_status, &Recording::default())
}

pub(super) fn recording_factory(variables: &[(&str, &str)]) -> (TestFactory, Recording) {
    let recording = Recording::default();
    let factory = build_factory(variables, 200, &recording);
    (factory, recording)
}

fn build_factory(
    variables: &[(&str, &str)],
    gcs_status: u16,
    recording: &Recording,
) -> TestFactory {
    let lookup: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(environment(variables));
    let http: Arc<dyn S3HttpClient> = Arc::new(FakeS3Http {
        status: Arc::clone(&recording.s3_status),
        requests: Arc::clone(&recording.s3_requests),
    });
    let clock: Arc<dyn Clock> = Arc::new(FakeS3Clock);
    let source: Arc<dyn EnvironmentSource> = Arc::new(environment(variables));
    let gcs = initialize(
        source,
        None,
        FakeGcsAuth,
        FakeGcsHttp { status: gcs_status },
    )
    .expect("the GCS base initializes without mock settings");
    TestFactory {
        environment: lookup,
        s3_http: http,
        s3_clock: clock,
        gcs,
        vantage: FakeCsvSender {
            status: Arc::clone(&recording.vantage_status),
            posts: Arc::clone(&recording.vantage_posts),
        },
        mavvrik: FakeMavvrikTransport {
            replies: Arc::clone(&recording.mavvrik_replies),
            requests: Arc::clone(&recording.mavvrik_requests),
        },
    }
}

pub(super) fn time_window() -> FocusTimeWindow {
    FocusTimeWindow {
        start_time: timestamp("2026-01-02T03:04:05Z"),
        end_time: timestamp("2026-01-02T04:05:06Z"),
        frequency: String::from("daily"),
    }
}

fn timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("the test timestamp is valid RFC 3339")
        .with_timezone(&Utc)
}

pub(super) fn mavvrik_reply(
    status: u16,
    body: &str,
    headers: Vec<(String, String)>,
) -> TransportResponse {
    TransportResponse {
        status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

pub(super) struct Recording {
    pub(super) s3_requests: Arc<Mutex<Vec<S3Request>>>,
    pub(super) s3_status: Arc<AtomicU16>,
    pub(super) vantage_posts: Arc<Mutex<Vec<RecordedPost>>>,
    pub(super) vantage_status: Arc<AtomicU16>,
    pub(super) mavvrik_requests: Arc<Mutex<Vec<RecordedMavvrikRequest>>>,
    pub(super) mavvrik_replies: Arc<Mutex<VecDeque<TransportResponse>>>,
}

impl Default for Recording {
    fn default() -> Self {
        Self {
            s3_requests: Arc::default(),
            s3_status: Arc::new(AtomicU16::new(200)),
            vantage_posts: Arc::default(),
            vantage_status: Arc::new(AtomicU16::new(200)),
            mavvrik_requests: Arc::default(),
            mavvrik_replies: Arc::default(),
        }
    }
}

#[derive(Debug)]
pub(super) struct RecordedPost {
    pub(super) url: String,
    pub(super) filename: Option<String>,
    pub(super) data: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct RecordedMavvrikRequest {
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct FakeS3Http {
    pub(super) status: Arc<AtomicU16>,
    pub(super) requests: Arc<Mutex<Vec<S3Request>>>,
}

impl S3HttpClient for FakeS3Http {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        self.requests.lock().unwrap().push(request);
        let status = self.status.load(Ordering::SeqCst);
        Box::pin(async move {
            Ok(S3Response {
                status,
                body: Vec::new(),
            })
        })
    }
}

#[derive(Debug)]
pub(super) struct FakeS3Clock;

impl Clock for FakeS3Clock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(1_767_323_045))
            .expect("the fixed signing time is representable")
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct FakeGcsAuth;

#[derive(Debug, thiserror::Error)]
#[error("the fake GCS auth failed")]
pub(super) struct GcsAuthFailed;

#[async_trait]
impl VertexAuth for FakeGcsAuth {
    type Error = GcsAuthFailed;

    async fn ensure_access_token_async(
        &self,
        _request: EnsureAccessToken,
    ) -> Result<AccessToken, GcsAuthFailed> {
        Ok(access_token())
    }

    fn ensure_access_token(
        &self,
        _request: EnsureAccessToken,
    ) -> Result<AccessToken, GcsAuthFailed> {
        Ok(access_token())
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, GcsAuthFailed> {
        Ok(TokenAndUrl {
            token: request
                .auth_header
                .unwrap_or_else(|| SecretString::from("fake-token")),
            url: String::new(),
        })
    }
}

fn access_token() -> AccessToken {
    AccessToken {
        token: SecretString::from("fake-token"),
        project_id: String::from("fake-project"),
    }
}

#[derive(Clone, Debug)]
pub(super) struct FakeGcsHttp {
    status: u16,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake GCS client failed")]
pub(super) struct GcsClientFailed;

#[async_trait]
impl GcsHttpClient for FakeGcsHttp {
    type Error = GcsClientFailed;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, GcsClientFailed> {
        Ok(GcsHttpResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: Vec::new(),
            url: request.url().to_owned(),
            elapsed: Duration::ZERO,
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct FakeCsvSender {
    pub(super) status: Arc<AtomicU16>,
    pub(super) posts: Arc<Mutex<Vec<RecordedPost>>>,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake CSV sender failed")]
pub(super) struct CsvSendFailed;

impl CsvSender for FakeCsvSender {
    type Error = CsvSendFailed;

    fn post_multipart(
        &self,
        post: MultipartPost,
    ) -> impl Future<Output = Result<u16, CsvSendFailed>> + Send {
        self.posts.lock().unwrap().push(RecordedPost {
            url: post.url.expose_secret().to_owned(),
            filename: post.file.file_name,
            data: post.file.data,
        });
        let status = self.status.load(Ordering::SeqCst);
        ready(Ok(status))
    }
}

#[derive(Clone, Debug)]
pub(super) struct FakeMavvrikTransport {
    pub(super) replies: Arc<Mutex<VecDeque<TransportResponse>>>,
    pub(super) requests: Arc<Mutex<Vec<RecordedMavvrikRequest>>>,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake Mavvrik transport failed")]
pub(super) struct MavvrikSendFailed;

impl Transport for FakeMavvrikTransport {
    type Error = MavvrikSendFailed;

    fn send(
        &self,
        request: TransportRequest,
    ) -> impl Future<Output = Result<TransportResponse, MavvrikSendFailed>> + Send {
        self.requests.lock().unwrap().push(RecordedMavvrikRequest {
            method: request.method,
            url: request.url,
            body: request.body,
        });
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("the fake Mavvrik transport has a scripted reply");
        ready(Ok(reply))
    }
}
