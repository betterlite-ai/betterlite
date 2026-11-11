use std::collections::BTreeMap;
use std::str;

use betterlite_rag::bedrock_ingestion::client::BedrockRagClients;
use betterlite_rag::bedrock_ingestion::resources::principal::normalize_principal_arn;
use betterlite_rag::bedrock_ingestion::settings::{BedrockRagSettings, VectorStoreConfig};
use betterlite_rag::bedrock_ingestion::state::BedrockRagState;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::input::{BucketCase, Input, PrincipalCase};
use super::support::{FixedClock, Outgoing, StaticResolver, Transport, fixed_now, json_response};
use crate::helpers::{HelperResult, OrHelperError as _};

struct KnowledgeBaseCase<'case> {
    vector: &'case VectorStoreConfig,
    environment: &'case BTreeMap<String, String>,
    kb_name: &'case str,
    knowledge_base_id: &'case str,
    role_arn: &'case str,
    collection_arn: &'case str,
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Principal { cases } => Ok(principal(cases)),
        Input::Detect {
            vector,
            environment,
            data_sources,
            data_source,
        } => detect(vector, environment, data_sources, data_source),
        Input::Bucket { cases } => bucket(cases),
        Input::CreateKnowledgeBase {
            vector,
            environment,
            kb_name,
            knowledge_base_id,
            role_arn,
            collection_arn,
        } => create_knowledge_base(&KnowledgeBaseCase {
            vector,
            environment,
            kb_name,
            knowledge_base_id,
            role_arn,
            collection_arn,
        }),
        Input::CreateDataSource {
            vector,
            environment,
            kb_name,
            data_source_id,
        } => create_data_source(vector, environment, kb_name, data_source_id),
    }
}

fn principal(cases: &[PrincipalCase]) -> Value {
    let arns: Vec<Value> = cases
        .iter()
        .map(|case| Value::String(normalize_principal_arn(&case.caller_arn, &case.account_id)))
        .collect();
    json!({"principal_arns": arns})
}

fn detect(
    vector: &VectorStoreConfig,
    environment: &BTreeMap<String, String>,
    data_sources: &Value,
    data_source: &Value,
) -> HelperResult {
    let settings =
        BedrockRagSettings::from_vector_store_config(vector, environment).or_value_error()?;
    let mut state = BedrockRagState::new(&settings);
    let transport = Transport::answering(vec![
        json_response(&json!({"dataSourceSummaries": data_sources}).to_string()),
        json_response(&data_source.to_string()),
    ]);
    let clients = BedrockRagClients {
        settings,
        resolver: Box::new(StaticResolver),
        clock: Box::new(FixedClock(fixed_now())),
        transport: transport.clone(),
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    runtime
        .block_on(state.auto_detect_config(&clients))
        .or_value_error()?;
    Ok(json!({
        "data_source_id": state.data_source_id,
        "s3_bucket": state.s3_bucket,
        "s3_prefix": state.s3_prefix,
    }))
}

fn bucket(cases: &[BucketCase]) -> HelperResult {
    let mut buckets: Vec<Value> = Vec::with_capacity(cases.len());
    for case in cases {
        let settings =
            BedrockRagSettings::from_vector_store_config(&case.vector, &case.environment)
                .or_value_error()?;
        let mut state = BedrockRagState::new(&settings);
        let transport = Transport::answering(vec![json_response("")]);
        let clients = BedrockRagClients {
            settings,
            resolver: Box::new(StaticResolver),
            clock: Box::new(FixedClock(fixed_now())),
            transport: transport.clone(),
        };
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let name = runtime
            .block_on(state.create_s3_bucket(&clients, &case.unique_id))
            .or_value_error()?;
        let sent = transport.sent();
        let constraint = sent
            .first()
            .and_then(|request| location_constraint(&request.body));
        buckets.push(json!({
            "name": name,
            "location_constraint": constraint,
            "created": state.created_resources.s3_bucket,
        }));
    }
    Ok(json!({"buckets": buckets}))
}

fn create_knowledge_base(case: &KnowledgeBaseCase<'_>) -> HelperResult {
    let settings = BedrockRagSettings::from_vector_store_config(case.vector, case.environment)
        .or_value_error()?;
    let mut state = BedrockRagState::new(&settings);
    let transport = Transport::answering(vec![
        json_response(
            &json!({"knowledgeBase": {"knowledgeBaseId": case.knowledge_base_id}}).to_string(),
        ),
        json_response(
            &json!({"knowledgeBase": {"knowledgeBaseId": case.knowledge_base_id, "status": "ACTIVE"}})
                .to_string(),
        ),
    ]);
    let clients = BedrockRagClients {
        settings,
        resolver: Box::new(StaticResolver),
        clock: Box::new(FixedClock(fixed_now())),
        transport: transport.clone(),
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let created = runtime
        .block_on(state.create_knowledge_base(
            &clients,
            case.kb_name,
            case.role_arn,
            case.collection_arn,
        ))
        .or_value_error()?;
    let sent = transport.sent();
    Ok(json!({
        "knowledge_base_id": created,
        "created": state.created_resources.knowledge_base,
        "create_params": sent.first().map(|request| request_params(&request.body)),
        "get_calls": polls(&sent),
    }))
}

fn create_data_source(
    vector: &VectorStoreConfig,
    environment: &BTreeMap<String, String>,
    kb_name: &str,
    data_source_id: &str,
) -> HelperResult {
    let settings =
        BedrockRagSettings::from_vector_store_config(vector, environment).or_value_error()?;
    let mut state = BedrockRagState::new(&settings);
    state.s3_bucket = settings.s3_bucket.clone();
    let transport = Transport::answering(vec![json_response(
        &json!({"dataSource": {"dataSourceId": data_source_id}}).to_string(),
    )]);
    let clients = BedrockRagClients {
        settings,
        resolver: Box::new(StaticResolver),
        clock: Box::new(FixedClock(fixed_now())),
        transport: transport.clone(),
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let created = runtime
        .block_on(state.create_data_source(&clients, kb_name))
        .or_value_error()?;
    let sent = transport.sent();
    Ok(json!({
        "data_source_id": created,
        "created": state.created_resources.data_source,
        "create_params": sent.first().map(|request| request_params(&request.body)),
    }))
}

fn request_params(body: &[u8]) -> Value {
    let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    match parsed {
        Value::Object(mut object) => {
            object.remove("clientToken");
            Value::Object(object)
        }
        other => other,
    }
}

fn polls(requests: &[Outgoing]) -> Vec<Value> {
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
