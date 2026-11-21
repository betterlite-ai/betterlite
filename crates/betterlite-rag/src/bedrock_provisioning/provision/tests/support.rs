use std::collections::{BTreeMap, VecDeque};
use std::iter;
use std::str;
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
use crate::bedrock_ingestion::state::BedrockRagState;
use crate::bedrock_provisioning::provision::BedrockProvisioner;

pub(crate) const UNIQUE_ID: &str = "0123abcd";
pub(crate) const ACCOUNT: &str = "123456789012";
pub(crate) const CALLER_ARN: &str = "arn:aws:iam::123456789012:role/caller";
pub(crate) const COLLECTION_ARN: &str = "arn:aws:aoss:us-west-2:123456789012:collection/abc";

#[derive(Debug, thiserror::Error)]
#[error("the scripted transport ran out of answers")]
pub(crate) struct ScriptError;

#[derive(Debug, Clone, Copy)]
pub(crate) struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_hours(490_896))
            .expect("the fixture instant is representable")
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FixedResolver;

#[async_trait]
impl RagCredentialResolver for FixedResolver {
    async fn credentials(
        &self,
        _settings: &AwsSettings,
        _region: &str,
    ) -> Result<Credentials, CredentialError> {
        Ok(Credentials {
            access_key_id: SecretString::from("AKIDRAGEXAMPLE".to_owned()),
            secret_access_key: SecretString::from("rag-example-secret".to_owned()),
            session_token: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScriptedTransport {
    answers: Arc<Mutex<VecDeque<AwsRagResponse>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl ScriptedTransport {
    pub(crate) fn new(answers: Vec<AwsRagResponse>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn operations(&self) -> Vec<String> {
        self.requests().iter().map(operation).collect()
    }
}

#[async_trait]
impl AwsRagTransport for ScriptedTransport {
    type Error = ScriptError;

    async fn send(&self, request: AwsSignedRequest) -> Result<AwsRagResponse, Self::Error> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: request.body,
            });
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or(ScriptError)
    }
}

// the AWS operation a recorded request belongs to, so a test can assert the call order
pub(crate) fn operation(request: &Recorded) -> String {
    if let Some(target) = header_target(&request.headers) {
        return target;
    }
    if let Some(action) = form_action(&request.body) {
        return action;
    }
    path_operation(&request.url, &request.method)
}

fn header_target(headers: &[(String, String)]) -> Option<String> {
    let (_, target) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("X-Amz-Target"))?;
    Some(target.rsplit('.').next().unwrap_or_default().to_owned())
}

fn form_action(body: &[u8]) -> Option<String> {
    let text = str::from_utf8(body).ok()?;
    let pair = text.split('&').next()?;
    let action = pair.strip_prefix("Action=")?;
    Some(action.to_owned())
}

fn path_operation(url: &str, method: &str) -> String {
    let read = method == "GET";
    if url.contains("/ingestionjobs/") {
        let name = if read {
            "GetIngestionJob"
        } else {
            "StartIngestionJob"
        };
        return name.to_owned();
    }
    if url.contains("/bedrock-kb-index") {
        return "CreateIndex".to_owned();
    }
    if url.contains("/datasources/") {
        let name = match method {
            "GET" => "GetDataSource",
            "POST" => "ListDataSources",
            _ => "CreateDataSource",
        };
        return name.to_owned();
    }
    if url.contains("/knowledgebases/") {
        let name = match method {
            "GET" => "GetKnowledgeBase",
            _ => "CreateKnowledgeBase",
        };
        return name.to_owned();
    }
    if read {
        return format!("GET {url}");
    }
    if url.ends_with('/') {
        return "CreateBucket".to_owned();
    }
    "PutObject".to_owned()
}

pub(crate) fn answer(body: &str) -> AwsRagResponse {
    AwsRagResponse {
        status: 200,
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn status(code: u16, body: &str) -> AwsRagResponse {
    AwsRagResponse {
        status: code,
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn repeated(response: AwsRagResponse, count: usize) -> Vec<AwsRagResponse> {
    iter::repeat_n(response, count).collect()
}

pub(crate) fn vector_config(text: &str) -> VectorStoreConfig {
    serde_json::from_str(text).expect("the fixture vector config is a JSON object")
}

pub(crate) fn new_provisioner(
    config: &str,
    transport: ScriptedTransport,
) -> BedrockProvisioner<'static, ScriptedTransport> {
    let settings =
        BedrockRagSettings::from_vector_store_config(&vector_config(config), &BTreeMap::new())
            .expect("the fixture settings are valid");
    let state = BedrockRagState::new(&settings);
    BedrockProvisioner {
        clients: BedrockRagClients {
            settings,
            resolver: Box::new(FixedResolver),
            clock: Box::new(FixedClock),
            transport,
        },
        state,
        ingest_name: None,
        unique_id: UNIQUE_ID.to_owned(),
        initialized: false,
    }
}

// the store tests run after the lazy initialization, with the ids it would have found
pub(crate) fn initialized(
    mut provisioner: BedrockProvisioner<'static, ScriptedTransport>,
) -> BedrockProvisioner<'static, ScriptedTransport> {
    provisioner.initialized = true;
    provisioner.state.s3_bucket = Some("bucket".to_owned());
    provisioner.state.data_source_id = Some("ds1".to_owned());
    provisioner
}
