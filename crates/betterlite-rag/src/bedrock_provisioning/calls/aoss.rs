use betterlite_context::aws_partition::get_aws_partition;
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
pub const DATA: &str = "data";
pub const AOSS_SERVICE: &str = "aoss";
pub const JSON_CONTENT_TYPE: &str = "application/x-amz-json-1.0";

const TARGET_PREFIX: &str = "OpenSearchServerless";
const VECTOR_SEARCH: &str = "VECTORSEARCH";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyRequest {
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

// BatchGetCollection takes either field, and an empty one is not a valid answer
#[derive(Debug, Clone, Copy)]
pub enum CollectionSelection<'selection> {
    Id(&'selection str),
    Name(&'selection str),
}

#[must_use]
pub fn aoss_url(region: &str) -> String {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    format!("https://aoss.{region}.{suffix}/")
}

pub fn create_policy<Policy>(
    region: &str,
    operation: &'static str,
    name: &str,
    kind: &str,
    policy: &Policy,
) -> Result<CallRequest, CallError>
where
    Policy: Serialize,
{
    let document = serde_json::to_string(policy)
        .map_err(|source| CallError::Serialize { operation, source })?;
    let request = PolicyRequest {
        name: name.to_owned(),
        policy_type: kind.to_owned(),
        policy: document,
        client_token: Uuid::new_v4().to_string(),
    };
    json_request(region, operation, &request)
}

pub fn create_collection(region: &str, name: &str) -> Result<CallRequest, CallError> {
    let request = CollectionRequest {
        name: name.to_owned(),
        kind: VECTOR_SEARCH.to_owned(),
        client_token: Uuid::new_v4().to_string(),
    };
    json_request(region, CREATE_COLLECTION, &request)
}

pub fn batch_get_collection(
    region: &str,
    selection: CollectionSelection<'_>,
) -> Result<CallRequest, CallError> {
    match selection {
        CollectionSelection::Id(id) => json_request(
            region,
            BATCH_GET_COLLECTION,
            &CollectionsById {
                ids: vec![id.to_owned()],
            },
        ),
        CollectionSelection::Name(name) => json_request(
            region,
            BATCH_GET_COLLECTION,
            &CollectionsByName {
                names: vec![name.to_owned()],
            },
        ),
    }
}

// an empty collectionDetails list is an answer that cannot be used, not an empty collection
pub fn first_collection(
    body: &[u8],
    operation: &'static str,
) -> Result<CollectionDetail, CallError> {
    let response: BatchGetCollectionResponse =
        serde_json::from_slice(body).map_err(|source| CallError::Json { operation, source })?;
    response
        .collection_details
        .into_iter()
        .next()
        .ok_or(CallError::MissingEntry {
            operation,
            field: "collectionDetails",
        })
}

fn json_request<Body>(
    region: &str,
    operation: &'static str,
    body: &Body,
) -> Result<CallRequest, CallError>
where
    Body: Serialize,
{
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
