use betterlite_rag::bedrock_ingestion::client::AwsRagResponse;
use serde_json::json;

use super::cases::Store;
use super::harness::answer;

const COLLECTION_POLLS: usize = 60;
const CREATED_COLLECTION: &str = r#"{"createCollectionDetail":{"id":"abc","status":"CREATING"}}"#;
const CREATED_KNOWLEDGE_BASE: &str =
    r#"{"knowledgeBase":{"knowledgeBaseId":"kb1","status":"CREATING"}}"#;
const ACTIVE_KNOWLEDGE_BASE: &str =
    r#"{"knowledgeBase":{"knowledgeBaseId":"kb1","status":"ACTIVE"}}"#;
const CREATED_DATA_SOURCE: &str = r#"{"dataSource":{"dataSourceId":"ds1"}}"#;
const DATA_SOURCE_SUMMARIES: &str = r#"{"dataSourceSummaries":[{"dataSourceId":"ds1"}]}"#;
const DETECTED_DATA_SOURCE: &str = r#"{"dataSource":{"dataSourceId":"ds1","dataSourceConfiguration":{"s3Configuration":{"bucketArn":"arn:aws:s3:::detected","inclusionPrefixes":["kb/"]}}}}"#;
pub(super) const STARTED_JOB: &str =
    r#"{"ingestionJob":{"ingestionJobId":"job1","status":"STARTING"}}"#;
const CALLER_IDENTITY: &str = "<GetCallerIdentityResponse><GetCallerIdentityResult><Arn>arn:aws:iam::123456789012:role/caller</Arn><Account>123456789012</Account></GetCallerIdentityResult></GetCallerIdentityResponse>";
const CREATED_ROLE: &str = "<CreateRoleResponse><CreateRoleResult><Role><Arn>arn:aws:iam::123456789012:role/litellm-bedrock-kb-created</Arn></Role></CreateRoleResult></CreateRoleResponse>";

fn responses(bodies: Vec<String>) -> Vec<AwsRagResponse> {
    let mut answers = Vec::new();
    for body in bodies {
        answers.push(answer(body));
    }
    answers
}

pub(super) fn collection_detail(status: &str) -> String {
    json!({
        "collectionDetails": [{
            "id": "abc",
            "status": status,
            "arn": "arn:aws:aoss:us-west-2:123456789012:collection/abc",
            "collectionEndpoint": "https://abc.us-west-2.aoss.amazonaws.com",
        }],
    })
    .to_string()
}

pub(super) fn collection_answers(statuses: &[String]) -> Vec<AwsRagResponse> {
    let mut bodies = vec![
        "{}".to_owned(),
        "{}".to_owned(),
        "{}".to_owned(),
        CREATED_COLLECTION.to_owned(),
    ];
    let fallback = statuses
        .last()
        .map_or_else(|| "CREATING".to_owned(), String::clone);
    for index in 0..COLLECTION_POLLS {
        let status = statuses
            .get(index)
            .map_or_else(|| fallback.clone(), String::clone);
        bodies.push(collection_detail(&status));
    }
    responses(bodies)
}

pub(super) fn detect_answers() -> Vec<AwsRagResponse> {
    responses(vec![
        DATA_SOURCE_SUMMARIES.to_owned(),
        DETECTED_DATA_SOURCE.to_owned(),
    ])
}

pub(super) fn store_answers(case: &Store) -> Vec<AwsRagResponse> {
    let mut answers = detect_answers();
    answers.push(answer(String::new()));
    answers.push(answer(STARTED_JOB.to_owned()));
    for job_status in &case.job_statuses {
        answers.push(answer(
            json!({"ingestionJob": {"ingestionJobId": "job1", "status": job_status}}).to_string(),
        ));
    }
    answers
}

pub(super) fn create_all_answers() -> Vec<AwsRagResponse> {
    responses(vec![
        CALLER_IDENTITY.to_owned(),
        String::new(),
        "{}".to_owned(),
        "{}".to_owned(),
        "{}".to_owned(),
        CREATED_COLLECTION.to_owned(),
        collection_detail("ACTIVE"),
        collection_detail("ACTIVE"),
        "{}".to_owned(),
        CREATED_ROLE.to_owned(),
        "{}".to_owned(),
        CREATED_KNOWLEDGE_BASE.to_owned(),
        ACTIVE_KNOWLEDGE_BASE.to_owned(),
        CREATED_DATA_SOURCE.to_owned(),
    ])
}
