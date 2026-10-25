#![expect(
    clippy::disallowed_types,
    reason = "SCIM discovery metadata and extensions retain unmodeled directory JSON"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimResource {
    pub schemas: Vec<String>,
    pub id: Option<String>,
    #[serde(rename = "externalId")]
    pub external_id: Option<String>,
    #[expect(
        clippy::disallowed_types,
        reason = "SCIM meta is an unmodeled passthrough field"
    )]
    pub meta: Option<Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimSchemaExtension {
    #[serde(alias = "schema")]
    pub schema_: String,
    pub required: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimResourceType {
    #[serde(default = "resource_type_schema")]
    pub schemas: Vec<String>,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub endpoint: String,
    #[serde(rename = "schema", alias = "schema_")]
    pub schema_: String,
    #[serde(
        default,
        rename = "schemaExtensions",
        skip_serializing_if = "Option::is_none"
    )]
    pub schema_extensions: Option<Vec<ScimSchemaExtension>>,
    #[expect(
        clippy::disallowed_types,
        reason = "SCIM meta is an unmodeled passthrough field"
    )]
    pub meta: Option<Value>,
}
fn resource_type_schema() -> Vec<String> {
    vec!["urn:ietf:params:scim:schemas:core:2.0:ResourceType".to_owned()]
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimSchemaAttribute {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, rename = "multiValued")]
    pub multi_valued: bool,
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default = "default_read_write")]
    pub mutability: String,
    #[serde(default = "default_returned")]
    pub returned: String,
    #[serde(default = "default_unique")]
    pub uniqueness: String,
    #[serde(
        default,
        rename = "subAttributes",
        skip_serializing_if = "Option::is_none"
    )]
    pub sub_attributes: Option<Vec<Self>>,
}
fn default_read_write() -> String {
    "readWrite".to_owned()
}
fn default_returned() -> String {
    "default".to_owned()
}
fn default_unique() -> String {
    "none".to_owned()
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimSchema {
    #[serde(default = "schema_schema")]
    pub schemas: Vec<String>,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub attributes: Vec<ScimSchemaAttribute>,
    #[expect(
        clippy::disallowed_types,
        reason = "SCIM meta is an unmodeled passthrough field"
    )]
    pub meta: Option<Value>,
}
fn schema_schema() -> Vec<String> {
    let schema = "urn:ietf:params:scim:schemas:core:2.0:Schema".to_owned();
    vec![schema]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScimFeature {
    pub supported: bool,
    pub max_operations: Option<u64>,
    pub max_payload_size: Option<u64>,
    pub max_results: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScimServiceProviderConfig {
    #[serde(default = "service_provider_schema")]
    pub schemas: Vec<String>,
    #[serde(default = "default_patch_feature")]
    pub patch: ScimFeature,
    #[serde(default = "default_unsupported_feature")]
    pub bulk: ScimFeature,
    #[serde(default = "default_unsupported_feature")]
    pub filter: ScimFeature,
    #[serde(default = "default_unsupported_feature")]
    pub change_password: ScimFeature,
    #[serde(default = "default_unsupported_feature")]
    pub sort: ScimFeature,
    #[serde(default = "default_unsupported_feature")]
    pub etag: ScimFeature,
    #[expect(
        clippy::disallowed_types,
        reason = "SCIM authentication schemes are unmodeled passthrough data"
    )]
    pub authentication_schemes: Option<Vec<Value>>,
    #[expect(
        clippy::disallowed_types,
        reason = "SCIM meta is an unmodeled passthrough field"
    )]
    pub meta: Option<Value>,
}
fn service_provider_schema() -> Vec<String> {
    vec!["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig".to_owned()]
}
const fn default_patch_feature() -> ScimFeature {
    ScimFeature {
        supported: true,
        max_operations: None,
        max_payload_size: None,
        max_results: None,
    }
}
const fn default_unsupported_feature() -> ScimFeature {
    ScimFeature {
        supported: false,
        max_operations: None,
        max_payload_size: None,
        max_results: None,
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScimListResponse {
    #[serde(default = "list_response_schema")]
    pub schemas: Vec<String>,
    pub total_results: u64,
    #[serde(default = "default_start_index")]
    pub start_index: Option<u64>,
    #[serde(default = "default_items_per_page")]
    pub items_per_page: Option<u64>,
    #[expect(
        clippy::disallowed_types,
        reason = "SCIM list resources are unmodeled passthrough records"
    )]
    #[serde(rename = "Resources")]
    pub resources: Vec<Value>,
}
fn list_response_schema() -> Vec<String> {
    vec!["urn:ietf:params:scim:api:messages:2.0:ListResponse".to_owned()]
}
#[expect(
    clippy::unnecessary_wraps,
    reason = "serde default must return the nullable wire field type"
)]
const fn default_start_index() -> Option<u64> {
    Some(1)
}
#[expect(
    clippy::unnecessary_wraps,
    reason = "serde default must return the nullable wire field type"
)]
const fn default_items_per_page() -> Option<u64> {
    Some(10)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimPlaceholderMergeResult {
    pub placeholder_user_id: String,
    pub merged_into_user_id: String,
    pub team_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiteLlmUserScimMetadata {
    pub given_name: Option<String>,
    pub family_name: Option<String>,
}
