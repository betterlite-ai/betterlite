use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_rag::bedrock_ingestion::client::clock::Clock;
use betterlite_rag::bedrock_ingestion::client::credentials::RagCredentialResolver;
use betterlite_rag::bedrock_ingestion::client::{AwsRagResponse, AwsRagTransport};
use secrecy::SecretString;

pub(super) const NOW_SECONDS: u64 = 1_767_225_600;

#[derive(Debug, Clone, Copy)]
pub(super) struct FixedClock(pub(super) SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

pub(super) fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .unwrap_or(UNIX_EPOCH)
}

#[derive(Debug, Clone)]
pub(super) struct StaticResolver;

#[async_trait]
impl RagCredentialResolver for StaticResolver {
    async fn credentials(
        &self,
        _settings: &AwsSettings,
        _region: &str,
    ) -> Result<Credentials, CredentialError> {
        Ok(Credentials {
            access_key_id: SecretString::from("AKIDRAGEXAMPLE".to_owned()),
            secret_access_key: SecretString::from("secret".to_owned()),
            session_token: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Outgoing {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
#[error("the oracle transport has no scripted answer")]
pub(super) struct NoAnswer;

#[derive(Debug, Clone, Default)]
pub(super) struct Transport {
    responses: Arc<Mutex<VecDeque<AwsRagResponse>>>,
    sent: Arc<Mutex<Vec<Outgoing>>>,
}

impl Transport {
    pub(super) fn answering(responses: Vec<AwsRagResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn sent(&self) -> Vec<Outgoing> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AwsRagTransport for Transport {
    type Error = NoAnswer;

    async fn send(&self, request: AwsSignedRequest) -> Result<AwsRagResponse, NoAnswer> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Outgoing {
                method: request.method,
                url: request.url,
                body: request.body,
            });
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or(NoAnswer)
    }
}

pub(super) fn json_response(body: &str) -> AwsRagResponse {
    AwsRagResponse {
        status: 200,
        body: body.as_bytes().to_vec(),
    }
}
