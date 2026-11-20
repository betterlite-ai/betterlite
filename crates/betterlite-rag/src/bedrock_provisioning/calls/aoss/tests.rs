#![cfg(test)]

use serde::Deserialize;

use super::{
    BATCH_GET_COLLECTION, CREATE_ACCESS_POLICY, CREATE_COLLECTION, CREATE_SECURITY_POLICY,
    CollectionSelection, CreateCollectionResponse, aoss_url, batch_get_collection,
    create_collection, create_policy, first_collection,
};
use crate::bedrock_provisioning::calls::CallRequest;
use crate::bedrock_provisioning::plan;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PolicyBody {
    name: String,
    #[serde(rename = "type")]
    policy_type: String,
    policy: String,
    client_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CollectionBody {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    client_token: String,
}

#[derive(Debug, Deserialize)]
struct Ids {
    ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Names {
    names: Vec<String>,
}

fn target(call: &CallRequest) -> Option<String> {
    call.headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("X-Amz-Target"))
        .map(|(_, value)| value.clone())
}

#[test]
fn call_create_security_policy_targets_the_aoss_endpoint() {
    let policy = plan::encryption_policy("kb");
    let call = create_policy(
        "us-west-2",
        CREATE_SECURITY_POLICY,
        "kb-enc",
        "encryption",
        &policy,
    )
    .unwrap();
    assert_eq!(call.operation, "CreateSecurityPolicy");
    assert_eq!(call.service, "aoss");
    assert_eq!(call.signing_region, "us-west-2");
    assert_eq!(call.method, "POST");
    assert_eq!(call.url, "https://aoss.us-west-2.amazonaws.com/");
    assert_eq!(
        target(&call).as_deref(),
        Some("OpenSearchServerless.CreateSecurityPolicy")
    );
    let content_type = call
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
        .map(|(_, value)| value.clone());
    assert_eq!(content_type.as_deref(), Some("application/x-amz-json-1.0"));
}

#[test]
fn call_create_security_policy_sends_the_document_and_a_uuid_token() {
    let call = create_policy(
        "us-west-2",
        CREATE_SECURITY_POLICY,
        "kb-enc",
        "encryption",
        &plan::encryption_policy("kb"),
    )
    .unwrap();
    let body: PolicyBody = serde_json::from_slice(&call.body).unwrap();
    assert_eq!(body.name, "kb-enc");
    assert_eq!(body.policy_type, "encryption");
    assert!(uuid::Uuid::parse_str(&body.client_token).is_ok());
    assert_eq!(
        body.policy,
        serde_json::to_string(&plan::encryption_policy("kb")).unwrap()
    );
}

#[test]
fn call_create_access_policy_sends_the_data_type() {
    let call = create_policy(
        "us-west-2",
        CREATE_ACCESS_POLICY,
        "kb-access",
        "data",
        &plan::data_access_policy("kb", "arn:aws:", "123456789012", "arn:aws:iam::1:root"),
    )
    .unwrap();
    assert_eq!(call.operation, "CreateAccessPolicy");
    assert_eq!(
        target(&call).as_deref(),
        Some("OpenSearchServerless.CreateAccessPolicy")
    );
    let body: PolicyBody = serde_json::from_slice(&call.body).unwrap();
    assert_eq!(body.name, "kb-access");
    assert_eq!(body.policy_type, "data");
    assert!(body.policy.starts_with("[{\"Rules\""));
}

#[test]
fn call_create_collection_sends_vectorsearch() {
    let call = create_collection("us-west-2", "litellm-kb-0123abcd").unwrap();
    assert_eq!(call.operation, CREATE_COLLECTION);
    assert_eq!(
        target(&call).as_deref(),
        Some("OpenSearchServerless.CreateCollection")
    );
    let body: CollectionBody = serde_json::from_slice(&call.body).unwrap();
    assert_eq!(body.name, "litellm-kb-0123abcd");
    assert_eq!(body.kind, "VECTORSEARCH");
    assert!(uuid::Uuid::parse_str(&body.client_token).is_ok());
}

#[test]
fn call_batch_get_collection_selects_by_id_or_name() {
    let by_id = batch_get_collection("us-west-2", CollectionSelection::Id("abc")).unwrap();
    assert_eq!(by_id.operation, BATCH_GET_COLLECTION);
    assert_eq!(
        target(&by_id).as_deref(),
        Some("OpenSearchServerless.BatchGetCollection")
    );
    let ids: Ids = serde_json::from_slice(&by_id.body).unwrap();
    assert_eq!(ids.ids, vec!["abc".to_owned()]);
    let by_name = batch_get_collection(
        "us-west-2",
        CollectionSelection::Name("litellm-kb-0123abcd"),
    )
    .unwrap();
    let names: Names = serde_json::from_slice(&by_name.body).unwrap();
    assert_eq!(names.names, vec!["litellm-kb-0123abcd".to_owned()]);
}

#[test]
fn call_aoss_url_follows_the_partition_suffix() {
    assert_eq!(
        aoss_url("us-east-1"),
        "https://aoss.us-east-1.amazonaws.com/"
    );
    assert_eq!(
        aoss_url("cn-north-1"),
        "https://aoss.cn-north-1.amazonaws.com.cn/"
    );
}

#[test]
fn call_create_collection_reads_the_detail_id() {
    let body = br#"{"createCollectionDetail":{"id":"abc","name":"kb","status":"CREATING"}}"#;
    let created: CreateCollectionResponse = serde_json::from_slice(body).unwrap();
    assert_eq!(
        created
            .create_collection_detail
            .and_then(|detail| detail.id),
        Some("abc".to_owned())
    );
}

#[test]
fn call_create_collection_reports_a_missing_detail() {
    let created: CreateCollectionResponse = serde_json::from_slice(b"{}").unwrap();
    assert_eq!(created.create_collection_detail, None);
}

#[test]
fn call_first_collection_reads_the_status_arn_and_endpoint() {
    let body = br#"{"collectionDetails":[{"id":"abc","status":"ACTIVE","arn":"arn:aws:aoss:us-west-2:1:collection/abc","collectionEndpoint":"https://abc.us-west-2.aoss.amazonaws.com"}]}"#;
    let detail = first_collection(body, BATCH_GET_COLLECTION).unwrap();
    assert_eq!(detail.status.as_deref(), Some("ACTIVE"));
    assert_eq!(
        detail.arn.as_deref(),
        Some("arn:aws:aoss:us-west-2:1:collection/abc")
    );
    assert_eq!(
        detail.collection_endpoint.as_deref(),
        Some("https://abc.us-west-2.aoss.amazonaws.com")
    );
}

#[test]
fn call_first_collection_reports_an_empty_list() {
    assert!(first_collection(br#"{"collectionDetails":[]}"#, BATCH_GET_COLLECTION).is_err());
}

#[test]
fn call_first_collection_reports_a_body_that_is_not_json() {
    assert!(first_collection(b"not json", BATCH_GET_COLLECTION).is_err());
}
