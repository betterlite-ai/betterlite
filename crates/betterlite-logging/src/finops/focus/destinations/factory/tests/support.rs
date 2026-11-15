use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use secrecy::SecretString;

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
use crate::mavvrik_delivery::transport::{Transport, TransportRequest, TransportResponse};
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
    let lookup: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(environment(variables));
    let http: Arc<dyn S3HttpClient> = Arc::new(FakeS3Http);
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
        vantage: FakeCsvSender,
        mavvrik: FakeMavvrikTransport,
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

#[derive(Debug)]
pub(super) struct FakeS3Http;

impl S3HttpClient for FakeS3Http {
    fn send(
        &self,
        _request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        Box::pin(async {
            Ok(S3Response {
                status: 200,
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

#[derive(Clone, Debug, Default)]
pub(super) struct FakeCsvSender;

#[derive(Debug, thiserror::Error)]
#[error("the fake CSV sender failed")]
pub(super) struct CsvSendFailed;

impl CsvSender for FakeCsvSender {
    type Error = CsvSendFailed;

    fn post_multipart(
        &self,
        _post: MultipartPost,
    ) -> impl Future<Output = Result<u16, CsvSendFailed>> + Send {
        ready(Ok(200))
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct FakeMavvrikTransport;

#[derive(Debug, thiserror::Error)]
#[error("the fake Mavvrik transport failed")]
pub(super) struct MavvrikSendFailed;

impl Transport for FakeMavvrikTransport {
    type Error = MavvrikSendFailed;

    fn send(
        &self,
        _request: TransportRequest,
    ) -> impl Future<Output = Result<TransportResponse, MavvrikSendFailed>> + Send {
        ready(Ok(TransportResponse {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
        }))
    }
}
