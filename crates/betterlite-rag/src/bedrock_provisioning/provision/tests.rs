#![cfg(test)]

use std::str;
use std::time::Duration;

use tokio::time::Instant;

use self::support::{
    ACCOUNT, CALLER_ARN, COLLECTION_ARN, ScriptedTransport, UNIQUE_ID, answer, new_provisioner,
    repeated, status,
};
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::provision::BedrockProvisioner;
use crate::bedrock_provisioning::provision::error::ProvisionError;

pub(crate) mod support;

const STS: &str = "<GetCallerIdentityResponse><GetCallerIdentityResult><Arn>arn:aws:iam::123456789012:role/caller</Arn><UserId>AIDA</UserId><Account>123456789012</Account></GetCallerIdentityResult></GetCallerIdentityResponse>";
const CREATED_COLLECTION: &str =
    r#"{"createCollectionDetail":{"id":"abc","name":"litellm-kb-0123abcd","status":"CREATING"}}"#;
const ACTIVE_BY_ID: &str = r#"{"collectionDetails":[{"id":"abc","status":"ACTIVE","arn":"arn:aws:aoss:us-west-2:123456789012:collection/abc"}]}"#;
const CREATING_BY_ID: &str = r#"{"collectionDetails":[{"id":"abc","status":"CREATING","arn":"arn:aws:aoss:us-west-2:123456789012:collection/abc"}]}"#;
const ACTIVE_BY_NAME: &str = r#"{"collectionDetails":[{"id":"abc","status":"ACTIVE","arn":"arn:aws:aoss:us-west-2:123456789012:collection/abc","collectionEndpoint":"https://abc.us-west-2.aoss.amazonaws.com"}]}"#;
const CREATED_ROLE: &str = "<CreateRoleResponse><CreateRoleResult><Role><Arn>arn:aws:iam::123456789012:role/litellm-bedrock-kb-0123abcd</Arn></Role></CreateRoleResult></CreateRoleResponse>";
const CREATED_KNOWLEDGE_BASE: &str =
    r#"{"knowledgeBase":{"knowledgeBaseId":"kb1","status":"CREATING"}}"#;
const ACTIVE_KNOWLEDGE_BASE: &str =
    r#"{"knowledgeBase":{"knowledgeBaseId":"kb1","status":"ACTIVE"}}"#;
const CREATED_DATA_SOURCE: &str = r#"{"dataSource":{"dataSourceId":"ds1"}}"#;
const CREATION_OPERATIONS: [&str; 14] = [
    "GetCallerIdentity",
    "CreateBucket",
    "CreateSecurityPolicy",
    "CreateSecurityPolicy",
    "CreateAccessPolicy",
    "CreateCollection",
    "BatchGetCollection",
    "BatchGetCollection",
    "CreateIndex",
    "CreateRole",
    "PutRolePolicy",
    "CreateKnowledgeBase",
    "GetKnowledgeBase",
    "CreateDataSource",
];

#[tokio::test(start_paused = true)]
async fn collection_sends_the_three_policies_then_creates_and_records() {
    let transport = ScriptedTransport::new(vec![
        answer("{}"),
        answer("{}"),
        answer("{}"),
        answer(CREATED_COLLECTION),
        answer(ACTIVE_BY_ID),
    ]);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let started = Instant::now();
    let (name, arn) = provisioner
        .create_opensearch_collection(UNIQUE_ID, ACCOUNT, CALLER_ARN)
        .await
        .unwrap();
    assert_eq!(name, "litellm-kb-0123abcd");
    assert_eq!(arn, COLLECTION_ARN);
    assert_eq!(
        provisioner.state.created_resources.opensearch_collection,
        Some("litellm-kb-0123abcd".to_owned())
    );
    assert_eq!(
        transport.operations(),
        vec![
            "CreateSecurityPolicy",
            "CreateSecurityPolicy",
            "CreateAccessPolicy",
            "CreateCollection",
            "BatchGetCollection",
        ]
    );
    assert_eq!(started.elapsed(), Duration::from_secs(60));
}

#[tokio::test(start_paused = true)]
async fn collection_reports_a_create_body_that_is_not_json() {
    let transport = ScriptedTransport::new(vec![
        answer("{}"),
        answer("{}"),
        answer("{}"),
        answer("not json"),
    ]);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport);
    let error = provisioner
        .create_opensearch_collection(UNIQUE_ID, ACCOUNT, CALLER_ARN)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProvisionError::Call(CallError::Json {
            operation: "CreateCollection",
            ..
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn collection_waits_through_sixty_checks_then_times_out() {
    let mut answers = vec![
        answer("{}"),
        answer("{}"),
        answer("{}"),
        answer(CREATED_COLLECTION),
    ];
    answers.extend(repeated(answer(CREATING_BY_ID), 60));
    let transport = ScriptedTransport::new(answers);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let started = Instant::now();
    let error = provisioner
        .create_opensearch_collection(UNIQUE_ID, ACCOUNT, CALLER_ARN)
        .await
        .unwrap_err();
    assert!(matches!(error, ProvisionError::CollectionTimeout));
    assert_eq!(started.elapsed(), Duration::from_secs(300));
    let batches = transport
        .operations()
        .iter()
        .filter(|operation| operation.as_str() == "BatchGetCollection")
        .count();
    assert_eq!(batches, 60);
}

#[tokio::test(start_paused = true)]
async fn collection_stops_waiting_when_the_status_is_active() {
    let transport = ScriptedTransport::new(vec![
        answer("{}"),
        answer("{}"),
        answer("{}"),
        answer(CREATED_COLLECTION),
        answer(CREATING_BY_ID),
        answer(ACTIVE_BY_ID),
    ]);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport);
    let started = Instant::now();
    provisioner
        .create_opensearch_collection(UNIQUE_ID, ACCOUNT, CALLER_ARN)
        .await
        .unwrap();
    assert_eq!(started.elapsed(), Duration::from_secs(65));
}

#[tokio::test(start_paused = true)]
async fn collection_records_nothing_when_the_create_call_fails() {
    let transport = ScriptedTransport::new(vec![
        answer("{}"),
        answer("{}"),
        answer("{}"),
        status(500, "boom"),
    ]);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport);
    let error = provisioner
        .create_opensearch_collection(UNIQUE_ID, ACCOUNT, CALLER_ARN)
        .await
        .unwrap_err();
    assert!(matches!(error, ProvisionError::Ingestion(_)));
    assert_eq!(
        provisioner.state.created_resources.opensearch_collection,
        None
    );
}

#[tokio::test(start_paused = true)]
async fn collection_reports_a_create_answer_without_a_detail() {
    let transport =
        ScriptedTransport::new(vec![answer("{}"), answer("{}"), answer("{}"), answer("{}")]);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let error = provisioner
        .create_opensearch_collection(UNIQUE_ID, ACCOUNT, CALLER_ARN)
        .await
        .unwrap_err();
    assert!(matches!(error, ProvisionError::Call(_)));
    assert_eq!(transport.operations().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn collection_reports_an_active_answer_without_an_arn() {
    let active = r#"{"collectionDetails":[{"id":"abc","status":"ACTIVE"}]}"#;
    let transport = ScriptedTransport::new(vec![
        answer("{}"),
        answer("{}"),
        answer("{}"),
        answer(CREATED_COLLECTION),
        answer(active),
    ]);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let error = provisioner
        .create_opensearch_collection(UNIQUE_ID, ACCOUNT, CALLER_ARN)
        .await
        .unwrap_err();
    assert!(matches!(error, ProvisionError::Call(_)));
}

#[tokio::test(start_paused = true)]
async fn collection_uses_the_partition_arn_prefix_in_the_access_policy() {
    let transport = ScriptedTransport::new(vec![
        answer("{}"),
        answer("{}"),
        answer("{}"),
        answer(CREATED_COLLECTION),
        answer(ACTIVE_BY_ID),
    ]);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"cn-north-1"}"#, transport.clone());
    provisioner
        .create_opensearch_collection(UNIQUE_ID, ACCOUNT, CALLER_ARN)
        .await
        .unwrap();
    let access = transport.requests().remove(2);
    let body = str::from_utf8(&access.body).unwrap();
    assert!(body.contains("arn:aws-cn:iam::123456789012:root"));
    assert!(body.contains("arn:aws-cn:iam::123456789012:role/caller"));
}

#[tokio::test(start_paused = true)]
async fn index_looks_the_collection_up_by_name_and_creates_it() {
    let transport = ScriptedTransport::new(vec![answer(ACTIVE_BY_NAME), answer("{}")]);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    provisioner
        .create_opensearch_index("litellm-kb-0123abcd")
        .await
        .unwrap();
    assert_eq!(
        transport.operations(),
        vec!["BatchGetCollection", "CreateIndex"]
    );
    let create = transport.requests().remove(1);
    assert_eq!(
        create.url,
        "https://abc.us-west-2.aoss.amazonaws.com/bedrock-kb-index"
    );
    assert_eq!(create.method, "PUT");
    let body = str::from_utf8(&create.body).unwrap();
    assert!(body.contains("\"knn.algo_param.ef_search\":512"));
}

#[tokio::test(start_paused = true)]
async fn index_retries_authorization_failures_and_waits_twenty_seconds() {
    let transport = ScriptedTransport::new(vec![
        answer(ACTIVE_BY_NAME),
        status(403, "{\"message\":\"security_exception: no permissions\"}"),
        status(403, "{\"message\":\"authorization_exception\"}"),
        answer("{}"),
    ]);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let started = Instant::now();
    provisioner
        .create_opensearch_index("litellm-kb-0123abcd")
        .await
        .unwrap();
    assert_eq!(started.elapsed(), Duration::from_secs(40));
    assert_eq!(transport.requests().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn index_returns_other_failures_at_once() {
    let transport = ScriptedTransport::new(vec![
        answer(ACTIVE_BY_NAME),
        status(400, "{\"message\":\"bad request\"}"),
    ]);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let started = Instant::now();
    let error = provisioner
        .create_opensearch_index("litellm-kb-0123abcd")
        .await
        .unwrap_err();
    assert!(matches!(error, ProvisionError::Ingestion(_)));
    assert_eq!(started.elapsed(), Duration::ZERO);
    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn index_does_not_retry_a_403_without_an_authorization_marker() {
    let transport = ScriptedTransport::new(vec![
        answer(ACTIVE_BY_NAME),
        status(403, "{\"message\":\"forbidden\"}"),
    ]);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let error = provisioner
        .create_opensearch_index("litellm-kb-0123abcd")
        .await
        .unwrap_err();
    assert!(matches!(error, ProvisionError::Ingestion(_)));
    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn index_reports_exhausted_retries_after_eight_attempts() {
    let mut answers = vec![answer(ACTIVE_BY_NAME)];
    answers.extend(repeated(
        status(403, "{\"message\":\"security_exception\"}"),
        8,
    ));
    let transport = ScriptedTransport::new(answers);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let started = Instant::now();
    let error = provisioner
        .create_opensearch_index("litellm-kb-0123abcd")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProvisionError::IndexRetriesExhausted {
            attempts: 8,
            status: 403,
            ..
        }
    ));
    assert_eq!(started.elapsed(), Duration::from_secs(160));
    assert_eq!(transport.requests().len(), 9);
}

#[tokio::test(start_paused = true)]
async fn index_returns_a_transport_failure_at_once() {
    let transport = ScriptedTransport::new(vec![answer(ACTIVE_BY_NAME)]);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let error = provisioner
        .create_opensearch_index("litellm-kb-0123abcd")
        .await
        .unwrap_err();
    assert!(matches!(error, ProvisionError::Ingestion(_)));
    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn index_rejects_a_collection_endpoint_that_is_not_https() {
    let insecure = r#"{"collectionDetails":[{"collectionEndpoint":"http://abc.us-west-2.aoss.amazonaws.com"}]}"#;
    let transport = ScriptedTransport::new(vec![answer(insecure)]);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    assert!(
        provisioner
            .create_opensearch_index("litellm-kb-0123abcd")
            .await
            .is_err()
    );
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn index_rejects_a_collection_endpoint_outside_the_aoss_host() {
    let elsewhere = r#"{"collectionDetails":[{"collectionEndpoint":"https://evil.example.com"}]}"#;
    let transport = ScriptedTransport::new(vec![answer(elsewhere)]);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    assert!(
        provisioner
            .create_opensearch_index("litellm-kb-0123abcd")
            .await
            .is_err()
    );
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn index_reports_a_missing_collection_endpoint() {
    let missing = r#"{"collectionDetails":[{"status":"ACTIVE"}]}"#;
    let transport = ScriptedTransport::new(vec![answer(missing)]);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport);
    assert!(
        provisioner
            .create_opensearch_index("litellm-kb-0123abcd")
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn index_reports_an_empty_collection_list() {
    let transport = ScriptedTransport::new(vec![answer(r#"{"collectionDetails":[]}"#)]);
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport);
    assert!(
        provisioner
            .create_opensearch_index("litellm-kb-0123abcd")
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn role_creates_the_role_records_the_name_and_attaches_the_policy() {
    let transport = ScriptedTransport::new(vec![answer(CREATED_ROLE), answer("{}")]);
    let mut provisioner = new_provisioner(
        r#"{"aws_region_name":"us-west-2","s3_bucket":"litellm-kb-0123abcd"}"#,
        transport.clone(),
    );
    let started = Instant::now();
    let arn = provisioner
        .create_bedrock_role(UNIQUE_ID, ACCOUNT, COLLECTION_ARN)
        .await
        .unwrap();
    assert_eq!(
        arn,
        "arn:aws:iam::123456789012:role/litellm-bedrock-kb-0123abcd"
    );
    assert_eq!(
        provisioner.state.created_resources.iam_role,
        Some("litellm-bedrock-kb-0123abcd".to_owned())
    );
    assert_eq!(transport.operations(), vec!["CreateRole", "PutRolePolicy"]);
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    let put = transport.requests().remove(1);
    let body = str::from_utf8(&put.body).unwrap();
    assert!(body.contains("PolicyName=litellm-bedrock-kb-0123abcd-policy"));
    assert!(body.contains("litellm-kb-0123abcd"));
}

#[tokio::test(start_paused = true)]
async fn role_requires_a_bucket() {
    let transport = ScriptedTransport::new(Vec::new());
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    let error = provisioner
        .create_bedrock_role(UNIQUE_ID, ACCOUNT, COLLECTION_ARN)
        .await
        .unwrap_err();
    assert!(matches!(error, ProvisionError::Ingestion(_)));
    assert!(transport.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn role_reports_a_create_role_failure_and_attaches_nothing() {
    let transport = ScriptedTransport::new(vec![status(400, "nope")]);
    let mut provisioner = new_provisioner(
        r#"{"aws_region_name":"us-west-2","s3_bucket":"bucket"}"#,
        transport.clone(),
    );
    let error = provisioner
        .create_bedrock_role(UNIQUE_ID, ACCOUNT, COLLECTION_ARN)
        .await
        .unwrap_err();
    assert!(matches!(error, ProvisionError::Ingestion(_)));
    assert_eq!(provisioner.state.created_resources.iam_role, None);
    assert_eq!(transport.operations(), vec!["CreateRole"]);
}

#[tokio::test(start_paused = true)]
async fn role_uses_the_partition_arn_prefix() {
    let transport = ScriptedTransport::new(vec![answer(CREATED_ROLE), answer("{}")]);
    let mut provisioner = new_provisioner(
        r#"{"aws_region_name":"cn-north-1","s3_bucket":"bucket"}"#,
        transport.clone(),
    );
    provisioner
        .create_bedrock_role(UNIQUE_ID, ACCOUNT, COLLECTION_ARN)
        .await
        .unwrap();
    let create = transport.requests().remove(0);
    let body = str::from_utf8(&create.body).unwrap();
    assert!(
        body.contains("arn%3Aaws-cn%3Abedrock%3Acn-north-1%3A123456789012%3Aknowledge-base%2F%2A")
    );
    assert_eq!(create.url, "https://iam.cn-north-1.amazonaws.com.cn/");
}

#[tokio::test(start_paused = true)]
async fn flow_uses_an_existing_knowledge_base_without_creating() {
    let transport = ScriptedTransport::new(vec![
        answer(r#"{"dataSourceSummaries":[{"dataSourceId":"ds1"}]}"#),
        answer(
            r#"{"dataSource":{"dataSourceId":"ds1","dataSourceConfiguration":{"s3Configuration":{"bucketArn":"arn:aws:s3:::detected","inclusionPrefixes":["kb/"]}}}}"#,
        ),
    ]);
    let mut provisioner = new_provisioner(
        r#"{"aws_region_name":"us-west-2","knowledge_base_id":"kb1"}"#,
        transport.clone(),
    );
    provisioner.ensure_config_initialized().await.unwrap();
    assert!(provisioner.initialized);
    assert_eq!(provisioner.state.data_source_id.as_deref(), Some("ds1"));
    assert_eq!(provisioner.state.s3_bucket.as_deref(), Some("detected"));
    assert_eq!(provisioner.state.s3_prefix, "kb/");
    assert_eq!(
        transport.operations(),
        vec!["ListDataSources", "GetDataSource"]
    );
}

#[tokio::test(start_paused = true)]
async fn flow_creates_every_resource_in_order() {
    let transport = ScriptedTransport::new(vec![
        answer(STS),
        answer(""),
        answer("{}"),
        answer("{}"),
        answer("{}"),
        answer(CREATED_COLLECTION),
        answer(ACTIVE_BY_ID),
        answer(ACTIVE_BY_NAME),
        answer("{}"),
        answer(CREATED_ROLE),
        answer("{}"),
        answer(CREATED_KNOWLEDGE_BASE),
        answer(ACTIVE_KNOWLEDGE_BASE),
        answer(CREATED_DATA_SOURCE),
    ]);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    provisioner.ensure_config_initialized().await.unwrap();
    assert_eq!(transport.operations(), CREATION_OPERATIONS);
    assert_created_resources(&provisioner);
}

fn assert_created_resources(provisioner: &BedrockProvisioner<'_, ScriptedTransport>) {
    assert_eq!(
        provisioner.clients.settings.knowledge_base_id.as_deref(),
        Some("kb1")
    );
    assert_eq!(provisioner.state.data_source_id.as_deref(), Some("ds1"));
    assert_eq!(
        provisioner.state.s3_bucket.as_deref(),
        Some("litellm-kb-0123abcd")
    );
    assert_eq!(
        provisioner.state.created_resources.s3_bucket.as_deref(),
        Some("litellm-kb-0123abcd")
    );
    assert_eq!(
        provisioner
            .state
            .created_resources
            .opensearch_collection
            .as_deref(),
        Some("litellm-kb-0123abcd")
    );
    assert_eq!(
        provisioner.state.created_resources.iam_role.as_deref(),
        Some("litellm-bedrock-kb-0123abcd")
    );
}

#[test]
fn flow_debug_names_the_state_and_the_unique_id() {
    let transport = ScriptedTransport::new(Vec::new());
    let provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport);
    let printed = format!("{provisioner:?}");
    assert!(printed.contains("BedrockProvisioner"));
    assert!(printed.contains(UNIQUE_ID));
    assert!(printed.contains("initialized: false"));
}

#[tokio::test(start_paused = true)]
async fn flow_keeps_the_flag_unset_after_a_failure() {
    let transport = ScriptedTransport::new(vec![status(500, "boom")]);
    let mut provisioner = new_provisioner(r#"{"aws_region_name":"us-west-2"}"#, transport.clone());
    assert!(provisioner.ensure_config_initialized().await.is_err());
    assert!(!provisioner.initialized);
    assert_eq!(transport.operations(), vec!["GetCallerIdentity"]);
}

#[tokio::test(start_paused = true)]
async fn flow_uses_the_ingest_name_and_the_configured_bucket() {
    let transport = ScriptedTransport::new(vec![
        answer(STS),
        answer("{}"),
        answer("{}"),
        answer("{}"),
        answer(CREATED_COLLECTION),
        answer(ACTIVE_BY_ID),
        answer(ACTIVE_BY_NAME),
        answer("{}"),
        answer(CREATED_ROLE),
        answer("{}"),
        answer(CREATED_KNOWLEDGE_BASE),
        answer(ACTIVE_KNOWLEDGE_BASE),
        answer(CREATED_DATA_SOURCE),
    ]);
    let mut provisioner = new_provisioner(
        r#"{"aws_region_name":"us-west-2","s3_bucket":"configured"}"#,
        transport.clone(),
    );
    provisioner.ingest_name = Some("custom".to_owned());
    provisioner.ensure_config_initialized().await.unwrap();
    assert!(
        !transport
            .operations()
            .iter()
            .any(|name| name == "CreateBucket")
    );
    assert_eq!(provisioner.state.s3_bucket.as_deref(), Some("configured"));
    let create = transport
        .requests()
        .into_iter()
        .find(|request| request.url.contains("/knowledgebases/"))
        .expect("the knowledge base create request is recorded");
    let body = str::from_utf8(&create.body).unwrap();
    assert!(body.contains("\"name\":\"custom\""));
}
