use serde::{Deserialize, Serialize};

use super::raw_object::RawObject;

schema_list! { ResourceTypeSchemas, "urn:ietf:params:scim:schemas:core:2.0:ResourceType" }
schema_list! { SchemaSchemas, "urn:ietf:params:scim:schemas:core:2.0:Schema" }
schema_list! { ProviderSchemas, "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig" }
// LiteLLM renames schema_ only in ResourceType.model_dump, which nested extensions never reach
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaExtension {
    #[serde(rename = "schema_")]
    pub schema: String,
    pub required: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceType {
    #[serde(default)]
    pub schemas: ResourceTypeSchemas,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub endpoint: String,
    #[serde(rename(serialize = "schema", deserialize = "schema_"))]
    pub schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_extensions: Option<Vec<SchemaExtension>>,
    pub meta: Option<RawObject>,
}
// only a top-level SCIMSchemaAttribute.model_dump drops a null subAttributes, and LiteLLM dumps them nested
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaAttribute {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub multi_valued: bool,
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub mutability: Mutability,
    #[serde(default)]
    pub returned: Returned,
    #[serde(default)]
    pub uniqueness: Uniqueness,
    pub sub_attributes: Option<Vec<Self>>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mutability {
    ReadOnly,
    #[default]
    ReadWrite,
    Immutable,
    WriteOnly,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Returned {
    Always,
    Never,
    #[default]
    Default,
    Request,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Uniqueness {
    #[default]
    None,
    Server,
    Global,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    #[serde(default)]
    pub schemas: SchemaSchemas,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub attributes: Vec<SchemaAttribute>,
    pub meta: Option<RawObject>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Copy, Default)]
#[serde(rename_all = "camelCase")]
pub struct Feature {
    pub supported: bool,
    pub max_operations: Option<u64>,
    pub max_payload_size: Option<u64>,
    pub max_results: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServiceProviderConfig {
    pub schemas: ProviderSchemas,
    pub patch: Feature,
    pub bulk: Feature,
    pub filter: Feature,
    pub change_password: Feature,
    pub sort: Feature,
    pub etag: Feature,
    pub authentication_schemes: Option<Vec<RawObject>>,
    pub meta: Option<RawObject>,
}
impl Default for ServiceProviderConfig {
    fn default() -> Self {
        let unsupported = Feature::default();
        Self {
            schemas: ProviderSchemas::default(),
            patch: Feature {
                supported: true,
                max_operations: None,
                max_payload_size: None,
                max_results: None,
            },
            bulk: unsupported,
            filter: unsupported,
            change_password: unsupported,
            sort: unsupported,
            etag: unsupported,
            authentication_schemes: None,
            meta: None,
        }
    }
}
