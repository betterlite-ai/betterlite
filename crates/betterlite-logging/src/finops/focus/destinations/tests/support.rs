use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use chrono::DateTime;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::focus::destinations::focus_gcs_destination::{
    FocusGcsDestination, GcsBucketClient, GcsMediaRequest, GcsMediaResponse, GcsSettings,
};
use crate::finops::focus::destinations::focus_s3_destination::{
    FocusS3Destination, S3Client, S3ClientOptions, S3PutObject, S3Settings,
};
use crate::focus_records::destinations::FocusTimeWindow;
use crate::gcs_client::gcs_storage_auth::{CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE, RequestHeaders};

pub(super) const BUCKET: &str = "focus-exports";
pub(super) const PREFIX: &str = "exports";
pub(super) const FILENAME: &str = "usage_20260102T030405Z_20260102T040405Z.csv";
pub(super) const CONTENT: &[u8] = b"ChargeCategory,Usage\n";
pub(super) const AUTHORIZATION: &str = "Bearer test-token";
pub(super) const SERVICE_ACCOUNT: &str = "/secrets/focus-service-account.json";
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

#[derive(Debug, thiserror::Error)]
#[error("the network is down")]
pub(super) struct NetworkDown;

#[derive(Clone, Debug)]
pub(super) struct SeenPut {
    pub(super) client_options: S3ClientOptions,
    pub(super) bucket_name: String,
    pub(super) object_key: String,
    pub(super) body: Vec<u8>,
    pub(super) content_type: &'static str,
}

#[derive(Clone, Default)]
pub(super) struct FakeS3Client {
    puts: Arc<Mutex<Vec<SeenPut>>>,
    failing: bool,
}

impl FakeS3Client {
    pub(super) fn failing() -> Self {
        Self {
            puts: Arc::default(),
            failing: true,
        }
    }

    pub(super) fn seen(&self) -> Vec<SeenPut> {
        self.puts.lock().unwrap().clone()
    }
}

impl S3Client for FakeS3Client {
    type Error = NetworkDown;

    fn put_object(
        &self,
        client_options: &S3ClientOptions,
        object: S3PutObject,
    ) -> impl Future<Output = Result<(), NetworkDown>> + Send {
        self.puts.lock().unwrap().push(SeenPut {
            client_options: client_options.clone(),
            bucket_name: object.bucket_name,
            object_key: object.object_key,
            body: object.body,
            content_type: object.content_type,
        });
        ready(if self.failing {
            Err(NetworkDown)
        } else {
            Ok(())
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct SeenHeaders {
    pub(super) bucket_name: String,
    pub(super) service_account_json: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct SeenPost {
    pub(super) url: String,
    pub(super) headers: RequestHeaders,
    pub(super) body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) enum GcsAnswer {
    Reply { status: u16, body: Vec<u8> },
    HeadersFailure,
    PostFailure,
}

#[derive(Clone)]
pub(super) struct FakeGcsClient {
    header_calls: Arc<Mutex<Vec<SeenHeaders>>>,
    posts: Arc<Mutex<Vec<SeenPost>>>,
    status: u16,
    response_body: Vec<u8>,
    header_failure: bool,
    post_failure: bool,
}

impl FakeGcsClient {
    pub(super) fn answering(answer: GcsAnswer) -> Self {
        let header_failure = matches!(answer, GcsAnswer::HeadersFailure);
        let post_failure = matches!(answer, GcsAnswer::PostFailure);
        let (status, response_body) = match answer {
            GcsAnswer::Reply { status, body } => (status, body),
            GcsAnswer::HeadersFailure | GcsAnswer::PostFailure => (200, Vec::new()),
        };
        Self {
            header_calls: Arc::default(),
            posts: Arc::default(),
            status,
            response_body,
            header_failure,
            post_failure,
        }
    }

    pub(super) fn header_calls(&self) -> Vec<SeenHeaders> {
        self.header_calls.lock().unwrap().clone()
    }

    pub(super) fn posts(&self) -> Vec<SeenPost> {
        self.posts.lock().unwrap().clone()
    }
}

impl Default for FakeGcsClient {
    fn default() -> Self {
        Self::answering(GcsAnswer::Reply {
            status: 200,
            body: Vec::new(),
        })
    }
}

impl GcsBucketClient for FakeGcsClient {
    type Error = NetworkDown;

    fn construct_request_headers(
        &self,
        bucket_name: &str,
        service_account_json: Option<&SecretString>,
    ) -> impl Future<Output = Result<RequestHeaders, NetworkDown>> + Send {
        self.header_calls.lock().unwrap().push(SeenHeaders {
            bucket_name: bucket_name.to_owned(),
            service_account_json: service_account_json
                .map(|value| value.expose_secret().to_owned()),
        });
        let outcome = if self.header_failure {
            Err(NetworkDown)
        } else {
            Ok(base_headers())
        };
        ready(outcome)
    }

    fn post_media(
        &self,
        request: GcsMediaRequest,
    ) -> impl Future<Output = Result<GcsMediaResponse, NetworkDown>> + Send {
        self.posts.lock().unwrap().push(SeenPost {
            url: request.url,
            headers: request.headers,
            body: request.body,
        });
        let outcome = if self.post_failure {
            Err(NetworkDown)
        } else {
            Ok(GcsMediaResponse {
                status: self.status,
                body: self.response_body.clone(),
            })
        };
        ready(outcome)
    }
}

fn base_headers() -> RequestHeaders {
    let mut headers = RequestHeaders::new();
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(AUTHORIZATION),
    );
    headers.insert(
        CONTENT_TYPE_HEADER.to_owned(),
        SecretString::from(JSON_CONTENT_TYPE),
    );
    headers
}

pub(super) fn s3_destination(
    prefix: &str,
    settings: S3Settings,
) -> (FocusS3Destination<FakeS3Client>, FakeS3Client) {
    let client = FakeS3Client::default();
    let destination = FocusS3Destination::new(client.clone(), prefix, settings).unwrap();
    (destination, client)
}

pub(super) fn gcs_destination(
    prefix: &str,
    settings: GcsSettings,
) -> (FocusGcsDestination<FakeGcsClient>, FakeGcsClient) {
    let client = FakeGcsClient::default();
    let destination = FocusGcsDestination::new(client.clone(), prefix, settings).unwrap();
    (destination, client)
}
