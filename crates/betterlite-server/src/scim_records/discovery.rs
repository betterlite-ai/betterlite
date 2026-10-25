use serde::{Deserialize, Serialize};

use super::RawObject;
use super::group::Group;
use super::user::User;

macro_rules! characteristic {
    ($name:ident, $case:literal { $($variants:tt)* }) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = $case)]
        pub enum $name { $($variants)* }
    };
}
schema_list! { ResourceTypeSchemas, "urn:ietf:params:scim:schemas:core:2.0:ResourceType" }
schema_list! { SchemaSchemas, "urn:ietf:params:scim:schemas:core:2.0:Schema" }
schema_list! { ProviderSchemas, "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig" }
schema_list! { ListSchemas, "urn:ietf:params:scim:api:messages:2.0:ListResponse" }
// LiteLLM renames schema_ only in ResourceType.model_dump, which nested extensions never reach
record! { SchemaExtension {
    #[serde(rename = "schema_")] pub schema: String,
    pub required: bool,
}}
record! { #[serde(rename_all = "camelCase")] ResourceType {
    #[serde(default)] pub schemas: ResourceTypeSchemas,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub endpoint: String,
    #[serde(rename(serialize = "schema", deserialize = "schema_"))] pub schema: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub schema_extensions: Option<Vec<SchemaExtension>>,
    pub meta: Option<RawObject>,
}}
// only a top-level SCIMSchemaAttribute.model_dump drops a null subAttributes, and LiteLLM dumps them nested
record! { #[serde(rename_all = "camelCase")] SchemaAttribute {
    pub name: String,
    #[serde(rename = "type")] pub kind: String,
    #[serde(default)] pub multi_valued: bool,
    pub description: Option<String>,
    #[serde(default)] pub required: bool,
    #[serde(default)] pub mutability: Mutability,
    #[serde(default)] pub returned: Returned,
    #[serde(default)] pub uniqueness: Uniqueness,
    pub sub_attributes: Option<Vec<Self>>,
}}
characteristic! { Mutability, "camelCase" { ReadOnly, #[default] ReadWrite, Immutable, WriteOnly } }
characteristic! { Returned, "lowercase" { Always, Never, #[default] Default, Request } }
characteristic! { Uniqueness, "lowercase" { #[default] None, Server, Global } }
record! { Schema {
    #[serde(default)] pub schemas: SchemaSchemas,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)] pub attributes: Vec<SchemaAttribute>,
    pub meta: Option<RawObject>,
}}
record! { #[derive(Copy, Default)] #[serde(rename_all = "camelCase")] Feature {
    pub supported: bool,
    pub max_operations: Option<u64>,
    pub max_payload_size: Option<u64>,
    pub max_results: Option<u64>,
}}
record! { #[serde(rename_all = "camelCase", default)] ServiceProviderConfig {
    pub schemas: ProviderSchemas,
    pub patch: Feature,
    pub bulk: Feature,
    pub filter: Feature,
    pub change_password: Feature,
    pub sort: Feature,
    pub etag: Feature,
    pub authentication_schemes: Option<Vec<RawObject>>,
    pub meta: Option<RawObject>,
}}
impl Default for ServiceProviderConfig {
    fn default() -> Self {
        let unsupported = Feature::default();
        Self {
            schemas: ProviderSchemas::default(),
            patch: Feature {
                supported: true,
                ..unsupported
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
// list responses are built by the handlers and only ever serialized
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse {
    pub schemas: ListSchemas,
    pub total_results: u64,
    pub start_index: Option<u64>,
    pub items_per_page: Option<u64>,
    #[serde(rename = "Resources")]
    pub resources: ListedResources,
}
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ListedResources {
    Users(Vec<User>),
    Groups(Vec<Group>),
}
record! { PlaceholderMergeResult {
    pub placeholder_user_id: String,
    pub merged_into_user_id: String,
    pub team_ids: Vec<String>,
}}
record! { #[serde(rename_all = "camelCase")] UserScimMetadata {
    pub given_name: Option<String>,
    pub family_name: Option<String>,
}}
