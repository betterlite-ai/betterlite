use std::collections::BTreeMap;

use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_context::aws_partition::get_aws_partition;
use betterlite_core::rag::ingestion::store::StoreRequest;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_rag::bedrock_ingestion::client::AwsRagResponse;
use betterlite_rag::bedrock_ingestion::state::CreatedResources;
use betterlite_rag::bedrock_provisioning::plan;
use betterlite_rag::bedrock_provisioning::provision::error::ProvisionError;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::answers::{STARTED_JOB, collection_answers, create_all_answers, detect_answers};
use super::cases::{Flow, Index, Input, Role, Store, failed};
use super::harness::{answer, form_fields, operation, provisioned, settings, status};

pub(super) fn run(runtime: &Runtime, input: &Input) -> HelperResult {
    match input {
        Input::Policies(case) => {
            let resolved = settings(&case.vector, &case.environment)?;
            let region = resolved.aws_region_name.as_str();
            let name = format!("litellm-kb-{}", case.unique_id);
            let arn_prefix = get_aws_partition(Some(region)).arn_prefix;
            Ok(json!({
                "encryption": plan::encryption_policy(&name),
                "network": plan::network_policy(&name),
                "access": plan::data_access_policy(
                    &name,
                    arn_prefix,
                    &case.account_id,
                    &case.caller_arn,
                ),
                "collection": name,
            }))
        }
        Input::Collection(case) => {
            let answers = collection_answers(&case.statuses);
            let (harness, mut provisioner) =
                provisioned(&case.vector, &case.environment, answers, &case.unique_id)?;
            let outcome = runtime.block_on(provisioner.create_opensearch_collection(
                &case.unique_id,
                &case.account_id,
                &case.caller_arn,
            ));
            let (_, calls) = harness.recorded();
            let created = provisioner
                .state
                .created_resources
                .opensearch_collection
                .clone();
            match outcome {
                Ok((name, arn)) => Ok(json!({
                    "outcome": "created",
                    "name": name,
                    "arn": arn,
                    "created": created,
                    "operations": calls,
                })),
                Err(ProvisionError::CollectionTimeout) => Ok(json!({
                    "outcome": "timeout",
                    "created": created,
                    "operations": calls,
                })),
                Err(error) => failed(error.to_string()),
            }
        }
        Input::Index(case) => {
            let answers = index_answers(case);
            let (harness, provisioner) =
                provisioned(&case.vector, &case.environment, answers, &case.unique_id)?;
            let outcome =
                runtime.block_on(provisioner.create_opensearch_index(&case.collection_name));
            let (requests, _) = harness.recorded();
            let attempts = requests
                .iter()
                .filter(|request| operation(request) == "CreateIndex")
                .count();
            let name = index_name(&requests);
            let body = if attempts == 0 {
                Value::Null
            } else {
                json!(plan::index::index_body())
            };
            match outcome {
                Ok(()) => Ok(json!({
                    "outcome": "created",
                    "attempts": attempts,
                    "index_name": name,
                    "index_body": body,
                })),
                Err(ProvisionError::IndexRetriesExhausted { .. }) => Ok(json!({
                    "outcome": "exhausted",
                    "attempts": attempts,
                    "index_name": name,
                    "index_body": body,
                })),
                Err(_error) => Ok(json!({
                    "outcome": "raised",
                    "attempts": attempts,
                    "index_name": name,
                    "index_body": body,
                })),
            }
        }
        Input::Role(case) => {
            let answers = role_answers(case);
            let (harness, mut provisioner) =
                provisioned(&case.vector, &case.environment, answers, &case.unique_id)?;
            let outcome = runtime.block_on(provisioner.create_bedrock_role(
                &case.unique_id,
                &case.account_id,
                &case.collection_arn,
            ));
            let (requests, calls) = harness.recorded();
            let created = provisioner.state.created_resources.iam_role.clone();
            match outcome {
                Ok(arn) => {
                    let create = form_fields(&requests[0]);
                    let attach = form_fields(&requests[1]);
                    Ok(json!({
                        "outcome": "created",
                        "role_arn": arn,
                        "created": created,
                        "operations": calls,
                        "role_name": create.get("RoleName"),
                        "policy_name": attach.get("PolicyName"),
                        "trust_document": create
                            .get("AssumeRolePolicyDocument")
                            .and_then(|text| serde_json::from_str::<Value>(text).ok()),
                        "permissions_document": attach
                            .get("PolicyDocument")
                            .and_then(|text| serde_json::from_str::<Value>(text).ok()),
                    }))
                }
                Err(error) => failed(error.to_string()),
            }
        }
        Input::Flow(case) => {
            let answers = flow_answers(&case.vector);
            let (harness, mut provisioner) =
                provisioned(&case.vector, &case.environment, answers, &case.unique_id)?;
            let outcome = runtime.block_on(provisioner.ensure_config_initialized());
            let (_, calls) = harness.recorded();
            let index_body = if calls.iter().any(|name| name == "CreateIndex") {
                json!(plan::index::index_body())
            } else {
                Value::Null
            };
            match outcome {
                Ok(()) => Ok(json!({
                    "outcome": "initialized",
                    "operations": calls,
                    "knowledge_base_id": provisioner.clients.settings.knowledge_base_id,
                    "data_source_id": provisioner.state.data_source_id,
                    "s3_bucket": provisioner.state.s3_bucket,
                    "s3_prefix": provisioner.state.s3_prefix,
                    "created_keys": created_keys(&provisioner.state.created_resources),
                    "index_body": index_body,
                })),
                Err(error) => failed(error.to_string()),
            }
        }
        Input::Store(case) => {
            let mut answers = detect_answers();
            answers.push(answer(String::new()));
            answers.push(answer(STARTED_JOB.to_owned()));
            for status in &case.job_statuses {
                answers.push(answer(
                    json!({"ingestionJob": {"ingestionJobId": "job1", "status": status}})
                        .to_string(),
                ));
            }
            let (harness, mut provisioner) =
                provisioned(&case.vector, &case.environment, answers, &case.unique_id)?;
            let outcome = runtime.block_on(provisioner.store(store_request(case)));
            let (requests, calls) = harness.recorded();
            match outcome {
                Ok((knowledge_base_id, file_key)) => Ok(json!({
                    "outcome": "stored",
                    "knowledge_base_id": knowledge_base_id,
                    "file_key": file_key,
                    "operations": calls,
                    "content_type": put_content_type(&requests),
                })),
                Err(error) => failed(error.to_string()),
            }
        }
    }
}

fn index_answers(case: &Index) -> Vec<AwsRagResponse> {
    let mut answers = vec![answer(
        json!({
            "collectionDetails": [{
                "id": "abc",
                "status": "ACTIVE",
                "collectionEndpoint": case.endpoint,
            }],
        })
        .to_string(),
    )];
    for marker in &case.failures {
        answers.push(status(403, json!({"message": marker}).to_string()));
    }
    answers.push(answer("{}".to_owned()));
    answers
}

fn role_answers(case: &Role) -> Vec<AwsRagResponse> {
    let role_arn = format!(
        "arn:aws:iam::{}:role/litellm-bedrock-kb-{}",
        case.account_id, case.unique_id
    );
    let created = format!(
        "<CreateRoleResponse><CreateRoleResult><Role><Arn>{role_arn}</Arn></Role></CreateRoleResult></CreateRoleResponse>"
    );
    vec![answer(created), answer("{}".to_owned())]
}

fn index_name(requests: &[AwsSignedRequest]) -> Option<String> {
    requests
        .iter()
        .find(|request| operation(request) == "CreateIndex")
        .and_then(|request| request.url.rsplit('/').next().map(str::to_owned))
}

fn put_content_type(requests: &[AwsSignedRequest]) -> Option<String> {
    for request in requests {
        if operation(request) == "PutObject" {
            return request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
                .map(|(_, value)| value.clone());
        }
    }
    None
}

fn store_request(case: &Store) -> StoreRequest<'_> {
    StoreRequest {
        file_content: case.content.as_deref().map(str::as_bytes),
        filename: case.filename.as_deref(),
        content_type: case.content_type.as_deref(),
        chunks: &[],
        embeddings: None,
        existing_file_id: None,
    }
}

fn created_keys(resources: &CreatedResources) -> Vec<&'static str> {
    let mut keys = Vec::new();
    if resources.s3_bucket.is_some() {
        keys.push("s3_bucket");
    }
    if resources.opensearch_collection.is_some() {
        keys.push("opensearch_collection");
    }
    if resources.iam_role.is_some() {
        keys.push("iam_role");
    }
    if resources.knowledge_base.is_some() {
        keys.push("knowledge_base");
    }
    if resources.data_source.is_some() {
        keys.push("data_source");
    }
    keys.sort_unstable();
    keys
}

fn flow_answers(vector: &BTreeMap<String, Value>) -> Vec<AwsRagResponse> {
    if vector.contains_key("vector_store_id") || vector.contains_key("knowledge_base_id") {
        return detect_answers();
    }
    create_all_answers()
}
