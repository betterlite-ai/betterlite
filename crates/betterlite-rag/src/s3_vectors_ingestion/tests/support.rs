use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::bedrock_ingestion::client::clock::SystemClock;
use crate::bedrock_ingestion::client::credentials::RagCredentialResolver;
use crate::bedrock_ingestion::client::{AwsRagResponse, AwsRagTransport, BedrockRagClients};
use crate::bedrock_ingestion::settings::{BedrockRagSettings, VectorStoreConfig};
use crate::s3_vectors_ingestion::S3VectorEntry;

pub(crate) const TEST_KEY: &str = "AKIDVECTORSEXAMPLE";
pub(crate) const TEST_SECRET: &str = "vectors-example-secret";
pub(crate) const REGION: &str = "us-west-2";

#[derive(Debug, thiserror::Error)]
#[error("the fake S3 Vectors transport failed")]
pub(crate) struct FakeTransportError;

#[derive(Debug, Clone, Copy)]
pub(crate) struct StaticResolver;

#[async_trait]
impl RagCredentialResolver for StaticResolver {
    async fn credentials(
        &self,
        _settings: &AwsSettings,
        _region: &str,
    ) -> Result<Credentials, CredentialError> {
        Ok(Credentials {
            access_key_id: SecretString::from(TEST_KEY.to_owned()),
            secret_access_key: SecretString::from(TEST_SECRET.to_owned()),
            session_token: None,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum FakeReply {
    Response(AwsRagResponse),
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedRequest {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct FakeTransport {
    responses: Arc<Mutex<VecDeque<FakeReply>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeTransport {
    pub(crate) fn new(responses: Vec<FakeReply>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AwsRagTransport for FakeTransport {
    type Error = FakeTransportError;

    async fn send(&self, request: AwsSignedRequest) -> Result<AwsRagResponse, Self::Error> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: request.body,
            });
        let reply = self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            Some(FakeReply::Response(response)) => Ok(response),
            Some(FakeReply::Failure) | None => Err(FakeTransportError),
        }
    }
}

pub(crate) fn rag_clients(transport: FakeTransport) -> BedrockRagClients<'static, FakeTransport> {
    let config: VectorStoreConfig = serde_json::from_str(
        r#"{"vector_store_id": "bucket:index", "aws_region_name": "us-west-2"}"#,
    )
    .expect("the fixture vector config is valid");
    let settings = BedrockRagSettings::from_vector_store_config(&config, &BTreeMap::new())
        .expect("the fixture settings are valid");
    BedrockRagClients {
        settings,
        resolver: Box::new(StaticResolver),
        clock: Box::new(SystemClock),
        transport,
    }
}

pub(crate) fn response(status: u16) -> FakeReply {
    FakeReply::Response(AwsRagResponse {
        status,
        body: Vec::new(),
    })
}

pub(crate) fn response_with_body(status: u16, body: &str) -> FakeReply {
    FakeReply::Response(AwsRagResponse {
        status,
        body: body.as_bytes().to_vec(),
    })
}

pub(crate) fn failure() -> FakeReply {
    FakeReply::Failure
}

pub(crate) fn header<'request>(
    request: &'request RecordedRequest,
    name: &str,
) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

pub(crate) fn body_hash(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordedBucketBody {
    pub(crate) vector_bucket_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordedIndexBody {
    pub(crate) vector_bucket_name: String,
    pub(crate) index_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordedCreateIndexBody {
    pub(crate) vector_bucket_name: String,
    pub(crate) index_name: String,
    pub(crate) data_type: String,
    pub(crate) dimension: i64,
    pub(crate) distance_metric: String,
    pub(crate) metadata_configuration: Option<RecordedMetadataConfiguration>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordedMetadataConfiguration {
    pub(crate) non_filterable_metadata_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordedPutVectorsBody {
    pub(crate) vector_bucket_name: String,
    pub(crate) index_name: String,
    pub(crate) vectors: Vec<S3VectorEntry>,
}
