use std::collections::BTreeMap;

use betterlite_rag::bedrock_ingestion::resources::principal::normalize_principal_arn;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use self::harness::{
    Harness, answer, data_source_params, location_constraint, polls, request_params, take_sent,
};
use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod harness;

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub(crate) enum Input {
    Principal {
        cases: Vec<Identity>,
    },
    Detect {
        vector: BTreeMap<String, Value>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        data_sources: Value,
        data_source: Value,
    },
    Bucket {
        cases: Vec<Bucket>,
    },
    CreateKnowledgeBase(CreateKnowledgeBaseCase),
    CreateDataSource {
        vector: BTreeMap<String, Value>,
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
    vector: BTreeMap<String, Value>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    unique_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateKnowledgeBaseCase {
    vector: BTreeMap<String, Value>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    kb_name: String,
    knowledge_base_id: String,
    role_arn: String,
    collection_arn: String,
}

fn detect(
    runtime: &Runtime,
    vector: &BTreeMap<String, Value>,
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
        let harness = Harness::build(&case.vector, &case.environment, vec![answer(String::new())])?;
        let clients = harness.clients();
        let mut state = harness.state;
        let name = runtime
            .block_on(state.create_s3_bucket(&clients, &case.unique_id))
            .or_value_error()?;
        let sent = take_sent(&harness.transport);
        buckets.push(json!({
            "name": name,
            "location_constraint": sent.first().and_then(|request| location_constraint(&request.body)),
            "created": state.created_resources.s3_bucket,
        }));
    }
    Ok(json!({"buckets": buckets}))
}

fn create_knowledge_base(runtime: &Runtime, case: &CreateKnowledgeBaseCase) -> HelperResult {
    let id = &case.knowledge_base_id;
    let harness = Harness::build(
        &case.vector,
        &case.environment,
        vec![
            answer(json!({"knowledgeBase": {"knowledgeBaseId": id}}).to_string()),
            answer(
                json!({"knowledgeBase": {"knowledgeBaseId": id, "status": "ACTIVE"}}).to_string(),
            ),
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
    let sent = take_sent(&harness.transport);
    Ok(json!({
        "knowledge_base_id": created,
        "created": state.created_resources.knowledge_base,
        "create_params": sent.first().map(request_params),
        "get_calls": polls(&sent),
    }))
}

fn create_data_source(
    runtime: &Runtime,
    vector: &BTreeMap<String, Value>,
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
    let sent = take_sent(&harness.transport);
    Ok(json!({
        "data_source_id": created,
        "created": state.created_resources.data_source,
        "create_params": sent.first().map(data_source_params),
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
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
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
