#![cfg(test)]

use serde::Deserialize;

use super::{
    BATCH_GET_COLLECTION, CREATE_COLLECTION, aoss_url, batch_get_collection_by_id,
    batch_get_collection_by_name, collection_details, create_access_policy, create_collection,
    create_security_policy, created_collection_id, first_collection,
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
    let call = create_security_policy("us-west-2", "kb-enc", "encryption", &policy).unwrap();
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
    let call = create_security_policy(
        "us-west-2",
        "kb-enc",
        "encryption",
        &plan::encryption_policy("kb"),
    )
    .unwrap();
    let body: PolicyBody = serde_json::from_slice(&call.body).unwrap();
    assert_eq!(body.name, "kb-enc");
    assert_eq!(body.policy_type, "encryption");
    assert!(uuid::Uuid::parse_str(&body.client_token).is_ok());
    let document: plan::EncryptionPolicy = serde_json::from_str(&body.policy).unwrap();
    assert_eq!(document, plan::encryption_policy("kb"));
}

#[test]
fn call_create_access_policy_sends_the_data_type() {
    let call = create_access_policy(
        "us-west-2",
        "kb-access",
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
    let by_id = batch_get_collection_by_id("us-west-2", "abc").unwrap();
    assert_eq!(by_id.operation, BATCH_GET_COLLECTION);
    assert_eq!(
        target(&by_id).as_deref(),
        Some("OpenSearchServerless.BatchGetCollection")
    );
    let ids: Ids = serde_json::from_slice(&by_id.body).unwrap();
    assert_eq!(ids.ids, vec!["abc".to_owned()]);
    let by_name = batch_get_collection_by_name("us-west-2", "litellm-kb-0123abcd").unwrap();
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
    assert_eq!(
        created_collection_id(body, CREATE_COLLECTION).unwrap(),
        "abc"
    );
}

#[test]
fn call_create_collection_reports_a_missing_detail() {
    assert!(created_collection_id(b"{}", CREATE_COLLECTION).is_err());
}

#[test]
fn call_collection_details_read_the_status_arn_and_endpoint() {
    let body = br#"{"collectionDetails":[{"id":"abc","status":"ACTIVE","arn":"arn:aws:aoss:us-west-2:1:collection/abc","collectionEndpoint":"https://abc.us-west-2.aoss.amazonaws.com"}]}"#;
    let details = collection_details(body, BATCH_GET_COLLECTION).unwrap();
    let detail = first_collection(&details, BATCH_GET_COLLECTION).unwrap();
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
fn call_collection_details_report_an_empty_list() {
    let details = collection_details(br#"{"collectionDetails":[]}"#, BATCH_GET_COLLECTION).unwrap();
    assert!(first_collection(&details, BATCH_GET_COLLECTION).is_err());
}

#[test]
fn call_collection_details_report_a_body_that_is_not_json() {
    assert!(collection_details(b"not json", BATCH_GET_COLLECTION).is_err());
}
