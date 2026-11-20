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
use betterlite_rag::bedrock_provisioning::provision::BedrockProvisioner;
use secrecy::SecretString;
use serde_json::Value;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

const ACCESS_KEY: &str = "AKIDORACLEB3457";
const SECRET_KEY: &str = "oracle-b3457-secret";
const SESSION_TOKEN: &str = "oracle-b3457-token";

// the URL path a call reaches, with the method AWS uses for that operation
const ROUTES: [(&str, &str, &str); 7] = [
    ("/ingestionjobs/", "GET", "GetIngestionJob"),
    ("/ingestionjobs/", "PUT", "StartIngestionJob"),
    ("/datasources/", "GET", "GetDataSource"),
    ("/datasources/", "POST", "ListDataSources"),
    ("/datasources/", "PUT", "CreateDataSource"),
    ("/knowledgebases/", "GET", "GetKnowledgeBase"),
    ("/knowledgebases/", "PUT", "CreateKnowledgeBase"),
];

#[derive(Debug, Clone, Copy)]
pub(super) struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct StaticResolver;

#[async_trait]
impl RagCredentialResolver for StaticResolver {
    async fn credentials(
        &self,
        _settings: &AwsSettings,
        _region: &str,
    ) -> Result<Credentials, CredentialError> {
        let credentials = Credentials {
            access_key_id: SecretString::from(ACCESS_KEY.to_owned()),
            secret_access_key: SecretString::from(SECRET_KEY.to_owned()),
            session_token: Some(SecretString::from(SESSION_TOKEN.to_owned())),
        };
        Ok(credentials)
    }
}

#[derive(Debug, Clone, Copy, Default, thiserror::Error)]
#[error("the oracle transport has no scripted answer")]
pub(super) struct NoAnswer;

#[derive(Debug, Clone, Default)]
pub(super) struct Transport {
    scripted: Arc<Mutex<VecDeque<AwsRagResponse>>>,
    sent: Arc<Mutex<Vec<AwsSignedRequest>>>,
}

impl Transport {
    fn answering(responses: Vec<AwsRagResponse>) -> Self {
        Self {
            scripted: Arc::new(Mutex::new(VecDeque::from(responses))),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl AwsRagTransport for Transport {
    type Error = NoAnswer;

    async fn send(&self, request: AwsSignedRequest) -> Result<AwsRagResponse, NoAnswer> {
        let answer = self
            .scripted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        answer.ok_or(NoAnswer)
    }
}

pub(super) fn take_sent(transport: &Transport) -> Vec<AwsSignedRequest> {
    match transport.sent.lock() {
        Ok(mut sent) => mem::take(&mut *sent),
        Err(poisoned) => mem::take(&mut *poisoned.into_inner()),
    }
}

pub(super) fn operation_names(requests: &[AwsSignedRequest]) -> Vec<String> {
    let mut names = Vec::new();
    for request in requests {
        names.push(operation(request));
    }
    names
}

pub(super) fn operation(request: &AwsSignedRequest) -> String {
    if let Some((_, target)) = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("X-Amz-Target"))
    {
        return target.rsplit('.').next().unwrap_or_default().to_owned();
    }
    if let Some(action) = form_action(&request.body) {
        return action;
    }
    route(request)
}

fn form_action(body: &[u8]) -> Option<String> {
    let text = str::from_utf8(body).ok()?;
    let first = text.split('&').next()?;
    first.strip_prefix("Action=").map(str::to_owned)
}

fn route(request: &AwsSignedRequest) -> String {
    for (fragment, method, name) in ROUTES {
        if request.method == method && request.url.contains(fragment) {
            return name.to_owned();
        }
    }
    if request.url.contains("/bedrock-kb-index") {
        return "CreateIndex".to_owned();
    }
    if request.url.ends_with('/') {
        return "CreateBucket".to_owned();
    }
    "PutObject".to_owned()
}

// the form fields of a query-protocol request, percent-decoded
pub(super) fn form_fields(request: &AwsSignedRequest) -> BTreeMap<String, String> {
    let text = str::from_utf8(&request.body).unwrap_or_default();
    let parsed = url::form_urlencoded::parse(text.as_bytes()).into_owned();
    parsed.collect()
}

pub(super) const fn answer(body: String) -> AwsRagResponse {
    AwsRagResponse {
        status: 200,
        body: body.into_bytes(),
    }
}

pub(super) const fn status(code: u16, body: String) -> AwsRagResponse {
    AwsRagResponse {
        status: code,
        body: body.into_bytes(),
    }
}

fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_hours(490_896))
        .unwrap_or(UNIX_EPOCH)
}

pub(super) fn settings(
    vector: &BTreeMap<String, Value>,
    environment: &BTreeMap<String, String>,
) -> Result<BedrockRagSettings, Box<HelperError>> {
    let text = serde_json::to_string(vector).or_value_error()?;
    let config: VectorStoreConfig = serde_json::from_str(&text).or_value_error()?;
    BedrockRagSettings::from_vector_store_config(&config, environment).or_value_error()
}

pub(super) struct Harness {
    pub(super) settings: BedrockRagSettings,
    pub(super) transport: Transport,
}

impl Harness {
    pub(super) fn build(
        vector: &BTreeMap<String, Value>,
        environment: &BTreeMap<String, String>,
        responses: Vec<AwsRagResponse>,
    ) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            settings: settings(vector, environment)?,
            transport: Transport::answering(responses),
        })
    }

    pub(super) fn recorded(&self) -> (Vec<AwsSignedRequest>, Vec<String>) {
        let requests = take_sent(&self.transport);
        let calls = operation_names(&requests);
        (requests, calls)
    }

    pub(super) fn provisioner(&self, unique_id: &str) -> BedrockProvisioner<'static, Transport> {
        BedrockProvisioner {
            clients: BedrockRagClients {
                settings: self.settings.clone(),
                resolver: Box::new(StaticResolver),
                clock: Box::new(FixedClock(fixed_now())),
                transport: self.transport.clone(),
            },
            state: BedrockRagState::new(&self.settings),
            ingest_name: None,
            unique_id: unique_id.to_owned(),
            initialized: false,
        }
    }
}

pub(super) fn provisioned(
    vector: &BTreeMap<String, Value>,
    environment: &BTreeMap<String, String>,
    responses: Vec<AwsRagResponse>,
    unique_id: &str,
) -> Result<(Harness, BedrockProvisioner<'static, Transport>), Box<HelperError>> {
    let harness = Harness::build(vector, environment, responses)?;
    let provisioner = harness.provisioner(unique_id);
    Ok((harness, provisioner))
}
