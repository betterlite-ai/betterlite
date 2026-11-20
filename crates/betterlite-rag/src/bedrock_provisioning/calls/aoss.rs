use betterlite_context::aws_partition::get_aws_partition;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::CallRequest;
use super::error::CallError;

pub const CREATE_SECURITY_POLICY: &str = "CreateSecurityPolicy";
pub const CREATE_ACCESS_POLICY: &str = "CreateAccessPolicy";
pub const CREATE_COLLECTION: &str = "CreateCollection";
pub const BATCH_GET_COLLECTION: &str = "BatchGetCollection";
pub const ENCRYPTION: &str = "encryption";
pub const NETWORK: &str = "network";
pub const AOSS_SERVICE: &str = "aoss";
pub const JSON_CONTENT_TYPE: &str = "application/x-amz-json-1.0";

const TARGET_PREFIX: &str = "OpenSearchServerless";
const DATA: &str = "data";
const VECTOR_SEARCH: &str = "VECTORSEARCH";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecurityPolicyRequest {
    name: String,
    #[serde(rename = "type")]
    policy_type: String,
    policy: String,
    client_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccessPolicyRequest {
    name: String,
    #[serde(rename = "type")]
    policy_type: String,
    policy: String,
    client_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CollectionRequest {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    client_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CollectionsById {
    ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CollectionsByName {
    names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCollectionResponse {
    pub create_collection_detail: Option<CreatedCollection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedCollection {
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchGetCollectionResponse {
    pub collection_details: Vec<CollectionDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDetail {
    pub status: Option<String>,
    pub arn: Option<String>,
    pub collection_endpoint: Option<String>,
}

#[must_use]
pub fn aoss_url(region: &str) -> String {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    format!("https://aoss.{region}.{suffix}/")
}

pub fn create_security_policy<Policy: Serialize>(
    region: &str,
    name: &str,
    policy_type: &str,
    policy: &Policy,
) -> Result<CallRequest, CallError> {
    let request = SecurityPolicyRequest {
        name: name.to_owned(),
        policy_type: policy_type.to_owned(),
        policy: super::document(policy, CREATE_SECURITY_POLICY)?,
        client_token: Uuid::new_v4().to_string(),
    };
    json_request(region, CREATE_SECURITY_POLICY, &request)
}

pub fn create_access_policy<Policy: Serialize>(
    region: &str,
    name: &str,
    policy: &Policy,
) -> Result<CallRequest, CallError> {
    let request = AccessPolicyRequest {
        name: name.to_owned(),
        policy_type: DATA.to_owned(),
        policy: super::document(policy, CREATE_ACCESS_POLICY)?,
        client_token: Uuid::new_v4().to_string(),
    };
    json_request(region, CREATE_ACCESS_POLICY, &request)
}

pub fn create_collection(region: &str, name: &str) -> Result<CallRequest, CallError> {
    let request = CollectionRequest {
        name: name.to_owned(),
        kind: VECTOR_SEARCH.to_owned(),
        client_token: Uuid::new_v4().to_string(),
    };
    json_request(region, CREATE_COLLECTION, &request)
}

pub fn batch_get_collection_by_id(
    region: &str,
    collection_id: &str,
) -> Result<CallRequest, CallError> {
    let request = CollectionsById {
        ids: vec![collection_id.to_owned()],
    };
    json_request(region, BATCH_GET_COLLECTION, &request)
}

pub fn batch_get_collection_by_name(
    region: &str,
    collection_name: &str,
) -> Result<CallRequest, CallError> {
    let request = CollectionsByName {
        names: vec![collection_name.to_owned()],
    };
    json_request(region, BATCH_GET_COLLECTION, &request)
}

pub fn created_collection_id(body: &[u8], operation: &'static str) -> Result<String, CallError> {
    let response: CreateCollectionResponse = parse(body, operation)?;
    response
        .create_collection_detail
        .and_then(|detail| detail.id)
        .ok_or(CallError::MissingEntry {
            operation,
            field: "createCollectionDetail.id",
        })
}

pub fn collection_details(
    body: &[u8],
    operation: &'static str,
) -> Result<BatchGetCollectionResponse, CallError> {
    parse(body, operation)
}

pub fn first_collection<'details>(
    response: &'details BatchGetCollectionResponse,
    operation: &'static str,
) -> Result<&'details CollectionDetail, CallError> {
    response
        .collection_details
        .first()
        .ok_or(CallError::MissingEntry {
            operation,
            field: "collectionDetails",
        })
}

fn parse<Response: DeserializeOwned>(
    body: &[u8],
    operation: &'static str,
) -> Result<Response, CallError> {
    serde_json::from_slice(body).map_err(|source| CallError::Json { operation, source })
}

fn json_request<Body: Serialize>(
    region: &str,
    operation: &'static str,
    body: &Body,
) -> Result<CallRequest, CallError> {
    let encoded =
        serde_json::to_vec(body).map_err(|source| CallError::Serialize { operation, source })?;
    Ok(CallRequest {
        operation,
        service: AOSS_SERVICE,
        signing_region: region.to_owned(),
        method: "POST",
        url: aoss_url(region),
        headers: vec![
            ("Content-Type".to_owned(), JSON_CONTENT_TYPE.to_owned()),
            (
                "X-Amz-Target".to_owned(),
                format!("{TARGET_PREFIX}.{operation}"),
            ),
        ],
        body: encoded,
    })
}

mod tests;
