use std::collections::{BTreeMap, VecDeque};
use std::mem;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_rag::bedrock_ingestion::client::clock::Clock;
use betterlite_rag::bedrock_ingestion::client::credentials::RagCredentialResolver;
use betterlite_rag::bedrock_ingestion::client::{
    AwsRagResponse, AwsRagTransport, BedrockRagClients,
};
use betterlite_rag::bedrock_ingestion::settings::{BedrockRagSettings, VectorStoreConfig};
use betterlite_rag::bedrock_ingestion::state::BedrockRagState;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Clone, Copy)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct StaticResolver;

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

#[derive(Debug, Clone, Copy, Default, thiserror::Error)]
#[error("the oracle transport has no scripted answer")]
pub(super) struct NoAnswer;

#[derive(Debug, Clone, Default)]
pub(super) struct Transport {
    responses: Arc<Mutex<VecDeque<AwsRagResponse>>>,
    sent: Arc<Mutex<Vec<AwsSignedRequest>>>,
}

impl Transport {
    fn answering(responses: Vec<AwsRagResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl AwsRagTransport for Transport {
    type Error = NoAnswer;

    async fn send(&self, request: AwsSignedRequest) -> Result<AwsRagResponse, NoAnswer> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or(NoAnswer)
    }
}

pub(super) fn take_sent(transport: &Transport) -> Vec<AwsSignedRequest> {
    match transport.sent.lock() {
        Ok(mut sent) => mem::take(&mut *sent),
        Err(poisoned) => mem::take(&mut *poisoned.into_inner()),
    }
}

pub(super) const fn answer(body: String) -> AwsRagResponse {
    AwsRagResponse {
        status: 200,
        body: body.into_bytes(),
    }
}

fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_hours(490_896))
        .unwrap_or(UNIX_EPOCH)
}

pub(super) struct Harness {
    pub(super) settings: BedrockRagSettings,
    pub(super) state: BedrockRagState,
    pub(super) transport: Transport,
}

impl Harness {
    pub(super) fn build(
        vector: &BTreeMap<String, Value>,
        environment: &BTreeMap<String, String>,
        responses: Vec<AwsRagResponse>,
    ) -> Result<Self, Box<HelperError>> {
        // the case files are YAML, which cannot fill the raw values of a VectorStoreConfig
        let text = serde_json::to_string(vector).or_value_error()?;
        let config: VectorStoreConfig = serde_json::from_str(&text).or_value_error()?;
        let settings =
            BedrockRagSettings::from_vector_store_config(&config, environment).or_value_error()?;
        Ok(Self {
            state: BedrockRagState::new(&settings),
            settings,
            transport: Transport::answering(responses),
        })
    }

    pub(super) fn clients(&self) -> BedrockRagClients<'static, Transport> {
        BedrockRagClients {
            settings: self.settings.clone(),
            resolver: Box::new(StaticResolver),
            clock: Box::new(FixedClock(fixed_now())),
            transport: self.transport.clone(),
        }
    }
}

pub(super) fn request_params(request: &AwsSignedRequest) -> Value {
    let mut value: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    if let Value::Object(object) = &mut value {
        object.remove("clientToken");
    }
    value
}

// boto3 exposes the URI knowledgeBaseId as a named parameter, so the comparison needs it back
pub(super) fn data_source_params(request: &AwsSignedRequest) -> Value {
    let mut value = request_params(request);
    let from_url = request
        .url
        .split("knowledgebases/")
        .nth(1)
        .and_then(|tail| tail.split('/').next())
        .map(str::to_owned);
    if let (Value::Object(object), Some(id)) = (&mut value, from_url) {
        object.insert("knowledgeBaseId".to_owned(), Value::String(id));
    }
    value
}

pub(super) fn polls(requests: &[AwsSignedRequest]) -> Vec<Value> {
    requests
        .iter()
        .filter(|request| request.method == "GET")
        .filter_map(|request| request.url.rsplit('/').next())
        .map(|id| json!({"knowledgeBaseId": id}))
        .collect()
}

pub(super) fn location_constraint(body: &[u8]) -> Option<String> {
    let text = str::from_utf8(body).ok()?;
    let open = "<LocationConstraint>";
    let start = text.find(open)?.saturating_add(open.len());
    let end = text.find("</LocationConstraint>")?;
    text.get(start..end).map(str::to_owned)
}
