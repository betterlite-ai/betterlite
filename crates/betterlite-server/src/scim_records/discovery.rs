use super::models::ScimValue;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimResource {
    pub schemas: Vec<String>,
    pub id: Option<String>,
    pub external_id: Option<String>,
    pub meta: Option<ScimValue>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimSchemaExtension {
    #[serde(rename = "schema", alias = "schema_")]
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
    #[serde(default, rename = "schemaExtensions")]
    pub schema_extensions: Option<Vec<ScimSchemaExtension>>,
    pub meta: Option<ScimValue>,
}
fn resource_type_schema() -> Vec<String> {
    let schema = "urn:ietf:params:scim:schemas:core:2.0:ResourceType".to_owned();
    vec![schema]
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimSchemaAttribute {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, rename = "multiValued")]
    pub multi_valued: bool,
    pub description: Option<String>,
    pub required: bool,
    #[serde(default = "default_read_write")]
    pub mutability: String,
    #[serde(default = "default_returned")]
    pub returned: String,
    #[serde(default = "default_unique")]
    pub uniqueness: String,
    #[serde(default, rename = "subAttributes")]
    pub sub_attributes: Option<Vec<Self>>,
}
fn default_read_write() -> String {
    let mut value = String::new();
    value.push_str("readWrite");
    value
}
fn default_returned() -> String {
    let mut value = String::new();
    value.push_str("default");
    value
}
fn default_unique() -> String {
    let mut value = String::new();
    value.push_str("none");
    value
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimSchema {
    #[serde(default = "schema_schema")]
    pub schemas: Vec<String>,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub attributes: Vec<ScimSchemaAttribute>,
    pub meta: Option<ScimValue>,
}
fn schema_schema() -> Vec<String> {
    let schema = "urn:ietf:params:scim:schemas:core:2.0:Schema".to_owned();
    vec![schema]
}
