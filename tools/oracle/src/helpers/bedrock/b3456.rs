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
use betterlite_rag::bedrock_ingestion::resources::principal::normalize_principal_arn;
use betterlite_rag::bedrock_ingestion::settings::{BedrockRagSettings, VectorStoreConfig};
use betterlite_rag::bedrock_ingestion::state::BedrockRagState;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub(crate) enum Input {
    Principal {
        cases: Vec<Identity>,
    },
    Detect {
        vector: VectorStoreConfig,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        data_sources: Value,
        data_source: Value,
    },
    Bucket {
        cases: Vec<Bucket>,
    },
    CreateKnowledgeBase(KnowledgeBase),
    CreateDataSource {
        vector: VectorStoreConfig,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        kb_name: String,
        data_source_id: String,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct Identity {
    caller_arn: String,
    account_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Bucket {
    vector: VectorStoreConfig,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    unique_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KnowledgeBase {
    vector: VectorStoreConfig,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    kb_name: String,
    knowledge_base_id: String,
    role_arn: String,
    collection_arn: String,
}

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
struct NoAnswer;

#[derive(Debug, Clone, Default)]
struct Transport {
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

    fn take(&self) -> Vec<AwsSignedRequest> {
        mem::take(&mut *self.sent.lock().unwrap_or_else(PoisonError::into_inner))
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

fn answer(body: String) -> AwsRagResponse {
    AwsRagResponse {
        status: 200,
        body: body.into_bytes(),
    }
}

fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_767_225_600))
        .unwrap_or(UNIX_EPOCH)
}

struct Harness {
    settings: BedrockRagSettings,
    state: BedrockRagState,
    transport: Transport,
}

impl Harness {
    fn build(
        vector: &VectorStoreConfig,
        environment: &BTreeMap<String, String>,
        responses: Vec<AwsRagResponse>,
    ) -> Result<Self, Box<HelperError>> {
        let settings =
            BedrockRagSettings::from_vector_store_config(vector, environment).or_value_error()?;
        Ok(Self {
            state: BedrockRagState::new(&settings),
            settings,
            transport: Transport::answering(responses),
        })
    }

    fn clients(&self) -> BedrockRagClients<'static, Transport> {
        BedrockRagClients {
            settings: self.settings.clone(),
            resolver: Box::new(StaticResolver),
            clock: Box::new(FixedClock(fixed_now())),
            transport: self.transport.clone(),
        }
    }
}

fn request_params(request: &AwsSignedRequest) -> Value {
    let mut value: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    if let Value::Object(object) = &mut value {
        object.remove("clientToken");
    }
    value
}

fn polls(requests: &[AwsSignedRequest]) -> Vec<Value> {
    requests
        .iter()
        .filter(|request| request.method == "GET")
        .filter_map(|request| request.url.rsplit('/').next())
        .map(|id| json!({"knowledgeBaseId": id}))
        .collect()
}

fn location_constraint(body: &[u8]) -> Option<String> {
    let text = str::from_utf8(body).ok()?;
    let open = "<LocationConstraint>";
    let start = text.find(open)?.saturating_add(open.len());
    let end = text.find("</LocationConstraint>")?;
    text.get(start..end).map(str::to_owned)
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    let built = Builder::new_current_thread().enable_all().build();
    built.or_value_error()
}

fn detect(
    runtime: &Runtime,
    vector: &VectorStoreConfig,
    environment: &BTreeMap<String, String>,
    data_sources: &Value,
    data_source: &Value,
) -> HelperResult {
    let harness = Harness::build(
        vector,
        environment,
        vec![
            answer(json!({"dataSourceSummaries": data_sources}).to_string()),
            answer(data_source.to_string()),
        ],
    )?;
    let clients = harness.clients();
    let mut state = harness.state;
    runtime
        .block_on(state.auto_detect_config(&clients))
        .or_value_error()?;
    Ok(json!({
        "data_source_id": state.data_source_id,
        "s3_bucket": state.s3_bucket,
        "s3_prefix": state.s3_prefix,
    }))
}

fn bucket(runtime: &Runtime, cases: &[Bucket]) -> HelperResult {
    let mut buckets: Vec<Value> = Vec::with_capacity(cases.len());
    for case in cases {
        let harness =
            Harness::build(&case.vector, &case.environment, vec![answer(String::new())])?;
        let clients = harness.clients();
        let mut state = harness.state;
        let name = runtime
            .block_on(state.create_s3_bucket(&clients, &case.unique_id))
            .or_value_error()?;
        let sent = harness.transport.take();
        buckets.push(json!({
            "name": name,
            "location_constraint": sent.first().and_then(|request| location_constraint(&request.body)),
            "created": state.created_resources.s3_bucket,
        }));
    }
    Ok(json!({"buckets": buckets}))
}

fn create_knowledge_base(runtime: &Runtime, case: &KnowledgeBase) -> HelperResult {
    let id = &case.knowledge_base_id;
    let harness = Harness::build(
        &case.vector,
        &case.environment,
        vec![
            answer(json!({"knowledgeBase": {"knowledgeBaseId": id}}).to_string()),
            answer(json!({"knowledgeBase": {"knowledgeBaseId": id, "status": "ACTIVE"}}).to_string()),
        ],
    )?;
    let clients = harness.clients();
    let mut state = harness.state;
    let created = runtime
        .block_on(state.create_knowledge_base(
            &clients,
            &case.kb_name,
            &case.role_arn,
            &case.collection_arn,
        ))
        .or_value_error()?;
    let sent = harness.transport.take();
    Ok(json!({
        "knowledge_base_id": created,
        "created": state.created_resources.knowledge_base,
        "create_params": sent.first().map(request_params),
        "get_calls": polls(&sent),
    }))
}

fn create_data_source(
    runtime: &Runtime,
    vector: &VectorStoreConfig,
    environment: &BTreeMap<String, String>,
    kb_name: &str,
    data_source_id: &str,
) -> HelperResult {
    let harness = Harness::build(
        vector,
        environment,
        vec![answer(
            json!({"dataSource": {"dataSourceId": data_source_id}}).to_string(),
        )],
    )?;
    let clients = harness.clients();
    let mut state = harness.state;
    state.s3_bucket = harness.settings.s3_bucket.clone();
    let created = runtime
        .block_on(state.create_data_source(&clients, kb_name))
        .or_value_error()?;
    let sent = harness.transport.take();
    Ok(json!({
        "data_source_id": created,
        "created": state.created_resources.data_source,
        "create_params": sent.first().map(request_params),
    }))
}

fn principals(cases: &[Identity]) -> Value {
    json!({
        "principal_arns": cases
            .iter()
            .map(|case| Value::String(normalize_principal_arn(&case.caller_arn, &case.account_id)))
            .collect::<Vec<Value>>(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = runtime()?;
        match input {
            Input::Principal { cases } => Ok(principals(cases)),
            Input::Detect {
                vector,
                environment,
                data_sources,
                data_source,
            } => detect(&runtime, vector, environment, data_sources, data_source),
            Input::Bucket { cases } => bucket(&runtime, cases),
            Input::CreateKnowledgeBase(case) => create_knowledge_base(&runtime, case),
            Input::CreateDataSource {
                vector,
                environment,
                kb_name,
                data_source_id,
            } => create_data_source(&runtime, vector, environment, kb_name, data_source_id),
        }
    }
}
