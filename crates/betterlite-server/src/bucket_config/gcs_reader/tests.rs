#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use betterlite_logging::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use betterlite_logging::gcs_mock::request::GcsRequest;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;

use super::{GcsBucketOpener, GcsReader, GcsReaderFactory};
use crate::bucket_config::object::BucketObjectReader;

const TOKEN: &str = "token";
const PROJECT: &str = "project";

#[derive(Debug, thiserror::Error)]
#[error("the test authentication failed")]
struct AuthFailure;

#[derive(Debug, thiserror::Error)]
#[error("the test transport failed")]
struct TransportFailure;

#[derive(Debug, Clone)]
struct FakeAuth {
    fail: bool,
}

impl FakeAuth {
    fn minting() -> Self {
        Self { fail: false }
    }

    fn failing() -> Self {
        Self { fail: true }
    }
}

#[async_trait]
impl VertexAuth for FakeAuth {
    type Error = AuthFailure;

    async fn ensure_access_token_async(
        &self,
        _request: EnsureAccessToken,
    ) -> Result<AccessToken, AuthFailure> {
        self.access_token()
    }

    fn ensure_access_token(&self, _request: EnsureAccessToken) -> Result<AccessToken, AuthFailure> {
        self.access_token()
    }

    fn get_token_and_url(&self, _request: GetTokenAndUrl) -> Result<TokenAndUrl, AuthFailure> {
        if self.fail {
            return Err(AuthFailure);
        }
        Ok(TokenAndUrl {
            token: SecretString::from(TOKEN.to_owned()),
            url: "https://storage.googleapis.com".to_owned(),
        })
    }
}

impl FakeAuth {
    fn access_token(&self) -> Result<AccessToken, AuthFailure> {
        if self.fail {
            return Err(AuthFailure);
        }
        Ok(AccessToken {
            token: SecretString::from(TOKEN.to_owned()),
            project_id: PROJECT.to_owned(),
        })
    }
}

#[derive(Debug, Clone)]
struct FakeHttp {
    status: u16,
    body: Vec<u8>,
    fail: bool,
}

impl FakeHttp {
    fn answering(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            body: body.to_vec(),
            fail: false,
        }
    }
}

#[async_trait]
impl GcsHttpClient for FakeHttp {
    type Error = TransportFailure;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, TransportFailure> {
        let url = request.url().to_owned();
        if self.fail {
            return Err(TransportFailure);
        }
        Ok(GcsHttpResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
            url,
            elapsed: Duration::ZERO,
        })
    }
}

fn environment(pairs: &[(&str, &str)]) -> Arc<dyn EnvironmentSource> {
    let values: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Arc::new(values)
}

#[tokio::test]
async fn a_200_answer_is_the_object_body() {
    let reader = GcsReader::initialize(
        Arc::new(BTreeMap::new()),
        "bucket",
        FakeAuth::minting(),
        FakeHttp::answering(200, b"model_list: []"),
    );

    assert_eq!(
        reader.read_object("config.yaml").await.as_deref(),
        Some(b"model_list: []".as_slice())
    );
}

#[tokio::test]
async fn a_non_200_answer_reads_as_missing() {
    let reader = GcsReader::initialize(
        Arc::new(BTreeMap::new()),
        "bucket",
        FakeAuth::minting(),
        FakeHttp::answering(404, b"no such object"),
    );

    assert!(reader.read_object("config.yaml").await.is_none());
}

#[tokio::test]
async fn a_transport_error_reads_as_missing() {
    let http = FakeHttp {
        status: 200,
        body: Vec::new(),
        fail: true,
    };
    let reader = GcsReader::initialize(
        Arc::new(BTreeMap::new()),
        "bucket",
        FakeAuth::minting(),
        http,
    );

    assert!(reader.read_object("config.yaml").await.is_none());
}

#[tokio::test]
async fn a_token_that_cannot_be_minted_reads_as_missing() {
    let reader = GcsReader::initialize(
        Arc::new(BTreeMap::new()),
        "bucket",
        FakeAuth::failing(),
        FakeHttp::answering(200, b"model_list: []"),
    );

    assert!(reader.read_object("config.yaml").await.is_none());
}

#[tokio::test]
async fn a_client_that_cannot_be_built_reads_nothing() {
    let reader = GcsReader::initialize(
        environment(&[("GCS_MOCK", "not-a-flag")]),
        "bucket",
        FakeAuth::minting(),
        FakeHttp::answering(200, b"model_list: []"),
    );

    assert!(reader.bucket.is_none());
    assert_eq!(reader.bucket_name, "bucket");
    assert!(reader.read_object("config.yaml").await.is_none());
}

#[rstest]
#[case::plain("bucket")]
#[case::nested("another/bucket")]
#[tokio::test]
async fn the_reader_factory_opens_a_reader_for_the_bucket(#[case] bucket_name: &str) {
    let factory = GcsReaderFactory {
        environment: Arc::new(BTreeMap::new()),
        auth: FakeAuth::minting(),
        http: FakeHttp::answering(200, b"model_list: []"),
    };

    let reader = factory.open(bucket_name);

    assert_eq!(
        reader.read_object("config.yaml").await.as_deref(),
        Some(b"model_list: []".as_slice())
    );
}

#[tokio::test]
async fn a_bucket_name_with_a_folder_is_passed_through() {
    let reader = GcsReader::initialize(
        Arc::new(BTreeMap::new()),
        "bucket/logs",
        FakeAuth::minting(),
        FakeHttp::answering(200, b"model_list: []"),
    );

    assert_eq!(reader.bucket_name, "bucket/logs");
    assert_eq!(
        reader.read_object("config.yaml").await.as_deref(),
        Some(b"model_list: []".as_slice())
    );
}
