use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;

use crate::bedrock_ingestion::client::clock::Clock;
use crate::bedrock_ingestion::client::credentials::RagCredentialResolver;
use crate::bedrock_ingestion::client::{AwsRagResponse, AwsRagTransport, BedrockRagClients};
use crate::bedrock_ingestion::settings::{BedrockRagSettings, VectorStoreConfig};

pub(crate) const TEST_KEY: &str = "AKIDRAGEXAMPLE";
pub(crate) const TEST_SECRET: &str = "rag-example-secret";
pub(crate) const NOW_SECONDS: u64 = 1_767_225_600;

#[derive(Debug, thiserror::Error)]
#[error("the fake transport failed")]
pub(crate) struct FakeTransportError;

#[derive(Debug, Clone, Copy)]
pub(crate) struct StaticClock {
    pub(crate) now: SystemTime,
}

impl Clock for StaticClock {
    fn now(&self) -> SystemTime {
        self.now
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StaticResolver {
    pub(crate) session_token: Option<SecretString>,
    seen: Arc<Mutex<Vec<(AwsSettings, String)>>>,
}

impl StaticResolver {
    pub(crate) fn new(session_token: Option<&str>) -> Self {
        Self {
            session_token: session_token.map(|token| SecretString::from(token.to_owned())),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn calls(&self) -> Vec<(AwsSettings, String)> {
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
            access_key_id: SecretString::from(TEST_KEY.to_owned()),
            secret_access_key: SecretString::from(TEST_SECRET.to_owned()),
            session_token: self.session_token.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedRequest {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FakeTransport {
    responses: Arc<Mutex<VecDeque<AwsRagResponse>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeTransport {
    pub(crate) fn new(responses: Vec<AwsRagResponse>) -> Self {
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
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or(FakeTransportError)
    }
}

pub(crate) fn json_response(body: &str) -> AwsRagResponse {
    AwsRagResponse {
        status: 200,
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn status_response(status: u16, body: &str) -> AwsRagResponse {
    AwsRagResponse {
        status,
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn vector_config(text: &str) -> VectorStoreConfig {
    serde_json::from_str(text).expect("the fixture vector config is a JSON object")
}

pub(crate) fn settings(text: &str) -> BedrockRagSettings {
    settings_in(text, &BTreeMap::new())
}

pub(crate) fn settings_in(
    text: &str,
    environment: &BTreeMap<String, String>,
) -> BedrockRagSettings {
    BedrockRagSettings::from_vector_store_config(&vector_config(text), environment)
        .expect("the fixture settings are valid")
}

pub(crate) fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .expect("the fixture instant is representable")
}

pub(crate) fn clients(
    settings: BedrockRagSettings,
    resolver: &StaticResolver,
    transport: FakeTransport,
) -> BedrockRagClients<'static, FakeTransport> {
    BedrockRagClients {
        settings,
        resolver: Box::new(resolver.clone()),
        clock: Box::new(StaticClock { now: fixed_now() }),
        transport,
    }
}
