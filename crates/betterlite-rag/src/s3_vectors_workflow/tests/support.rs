use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;
use serde::Deserialize;

use crate::bedrock_ingestion::client::clock::Clock;
use crate::bedrock_ingestion::client::credentials::RagCredentialResolver;
use crate::bedrock_ingestion::client::{AwsRagResponse, AwsRagTransport};
use crate::s3_vectors_ingestion::{S3VectorEntry, VectorIndexConfig};
use crate::s3_vectors_workflow::dimension::{EmbedError, RagEmbedder};
use crate::s3_vectors_workflow::workflow::S3VectorsApi;

pub(super) const TEST_KEY: &str = "AKIDVECTORSEXAMPLE";
pub(super) const UNIQUE_ID: &str = "abcd1234";

#[derive(Debug, thiserror::Error)]
#[error("the scripted S3 Vectors API failed")]
pub(super) struct ScriptedFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    Bucket,
    Index,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Call {
    Bucket(String),
    Index {
        bucket_name: String,
        index_name: String,
        dimension: i64,
        distance_metric: String,
        non_filterable_metadata_keys: Vec<String>,
    },
    Vectors {
        bucket_name: String,
        index_name: String,
        vectors: Vec<S3VectorEntry>,
    },
}

#[derive(Debug, Clone, Default)]
pub(super) struct FakeApi {
    calls: Arc<Mutex<Vec<Call>>>,
    failures: Arc<Mutex<Vec<Failure>>>,
}

impl FakeApi {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn failing(failures: Vec<Failure>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            failures: Arc::new(Mutex::new(failures)),
        }
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    fn next_failure(&self, operation: Failure) -> Result<(), ScriptedFailure> {
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        if failures.first() != Some(&operation) {
            return Ok(());
        }
        failures.remove(0);
        drop(failures);
        Err(ScriptedFailure)
    }
}

#[async_trait]
impl S3VectorsApi for FakeApi {
    type Error = ScriptedFailure;

    async fn ensure_bucket(&self, bucket_name: &str) -> Result<(), Self::Error> {
        self.record(Call::Bucket(bucket_name.to_owned()));
        self.next_failure(Failure::Bucket)
    }

    async fn ensure_index(&self, index: &VectorIndexConfig<'_>) -> Result<(), Self::Error> {
        self.record(Call::Index {
            bucket_name: index.bucket_name.to_owned(),
            index_name: index.index_name.to_owned(),
            dimension: index.dimension,
            distance_metric: index.distance_metric.to_owned(),
            non_filterable_metadata_keys: index.non_filterable_metadata_keys.to_vec(),
        });
        self.next_failure(Failure::Index)
    }

    async fn put_vectors(
        &self,
        bucket_name: &str,
        index_name: &str,
        vectors: &[S3VectorEntry],
    ) -> Result<(), Self::Error> {
        self.record(Call::Vectors {
            bucket_name: bucket_name.to_owned(),
            index_name: index_name.to_owned(),
            vectors: vectors.to_vec(),
        });
        Ok(())
    }
}

pub(super) type EmbedCalls = Vec<(String, Vec<String>)>;

#[derive(Debug, Clone)]
pub(super) struct FakeEmbedder {
    calls: Arc<Mutex<EmbedCalls>>,
    answer: Vec<Vec<f64>>,
    failing: bool,
}

impl FakeEmbedder {
    pub(super) fn new(answer: Vec<Vec<f64>>, failing: bool) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            answer,
            failing,
        }
    }

    pub(super) fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl RagEmbedder for FakeEmbedder {
    async fn embed(&self, model: &str, input: Vec<String>) -> Result<Vec<Vec<f64>>, EmbedError> {
        self.calls.lock().unwrap().push((model.to_owned(), input));
        if self.failing {
            return Err(EmbedError::Failed {
                model: model.to_owned(),
                message: "the scripted embedder failed".to_owned(),
            });
        }
        Ok(self.answer.clone())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_hours(490_896))
            .unwrap()
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct StaticResolver {
    seen: Arc<Mutex<Vec<(AwsSettings, String)>>>,
}

impl StaticResolver {
    pub(super) fn calls(&self) -> Vec<(AwsSettings, String)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RagCredentialResolver for StaticResolver {
    async fn credentials(
        &self,
        settings: &AwsSettings,
        region: &str,
    ) -> Result<Credentials, CredentialError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((settings.clone(), region.to_owned()));
        Ok(Credentials {
            access_key_id: settings
                .access_key_id
                .clone()
                .unwrap_or_else(|| SecretString::from(TEST_KEY.to_owned())),
            secret_access_key: settings
                .secret_access_key
                .clone()
                .unwrap_or_else(|| SecretString::from("vectors-example-secret".to_owned())),
            session_token: settings.session_token.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct RecordedRequest {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RecordingTransport {
    replies: Arc<Mutex<VecDeque<AwsRagResponse>>>,
    sent: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl RecordingTransport {
    pub(super) fn answering(replies: Vec<AwsRagResponse>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn sent(&self) -> Vec<RecordedRequest> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl AwsRagTransport for RecordingTransport {
    type Error = ScriptedFailure;

    async fn send(&self, request: AwsSignedRequest) -> Result<AwsRagResponse, ScriptedFailure> {
        self.sent.lock().unwrap().push(RecordedRequest {
            url: request.url.clone(),
            headers: request.headers.clone(),
            body: request.body,
        });
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(ScriptedFailure)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PutVectorsBody {
    pub(super) index_name: String,
    pub(super) vectors: Vec<S3VectorEntry>,
}
