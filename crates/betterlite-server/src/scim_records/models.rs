use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub(super) const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    User,
    Group,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DecodedScimResource {
    User(User),
    Group(Group),
    Patch(Patch),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScimResource {
    pub schemas: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct User {
    pub schemas: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "externalId")]
    pub external_id: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
    #[serde(default, rename = "userName")]
    pub user_name: Option<String>,
    #[serde(default)]
    pub name: Option<UserName>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub active: bool,
    #[serde(skip)]
    pub active_present: bool,
    #[serde(default)]
    pub emails: Option<Vec<UserEmail>>,
    #[serde(default)]
    pub groups: Option<Vec<UserGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entitlements: Option<Vec<ScimMultiValuedAttribute>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<ScimMultiValuedAttribute>>,
    #[serde(
        default,
        rename = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_user: Option<ScimEnterpriseUser>,
}

impl<'de> Deserialize<'de> for User {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Input {
            schemas: Vec<String>,
            #[serde(default)]
            id: Option<String>,
            #[serde(default, rename = "externalId")]
            external_id: Option<String>,
            #[serde(default)]
            meta: Option<Value>,
            #[serde(default, rename = "userName")]
            user_name: Option<String>,
            #[serde(default)]
            name: Option<UserName>,
            #[serde(default, rename = "displayName")]
            display_name: Option<String>,
            #[serde(default)]
            active: Option<bool>,
            #[serde(default)]
            emails: Option<Vec<UserEmail>>,
            #[serde(default)]
            groups: Option<Vec<UserGroup>>,
            #[serde(default)]
            entitlements: Option<Vec<ScimMultiValuedAttribute>>,
            #[serde(default)]
            roles: Option<Vec<ScimMultiValuedAttribute>>,
            #[serde(
                default,
                rename = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
            )]
            enterprise_user: Option<ScimEnterpriseUser>,
        }
        let input = Input::deserialize(deserializer)?;
        Ok(Self {
            schemas: input.schemas,
            id: input.id,
            external_id: input.external_id,
            meta: input.meta,
            user_name: input.user_name,
            name: input.name,
            display_name: input.display_name,
            active_present: input.active.is_some(),
            active: input.active.unwrap_or(true),
            emails: input.emails,
            groups: input.groups,
            entitlements: input.entitlements,
            roles: input.roles,
            enterprise_user: input.enterprise_user,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserName {
    #[serde(default)]
    pub family_name: Option<String>,
    #[serde(default)]
    pub given_name: Option<String>,
    #[serde(default)]
    pub formatted: Option<String>,
    #[serde(default)]
    pub middle_name: Option<String>,
    #[serde(default)]
    pub honorific_prefix: Option<String>,
    #[serde(default)]
    pub honorific_suffix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEmail {
    pub value: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub primary: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserGroup {
    pub value: String,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default = "default_direct", rename = "type")]
    pub kind: Option<String>,
}
fn default_direct() -> Option<String> {
    Some("direct".to_owned())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScimMultiValuedAttribute {
    Object {
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        display: Option<String>,
        #[serde(default, rename = "type")]
        kind: Option<String>,
        #[serde(default)]
        primary: Option<bool>,
        #[serde(flatten)]
        extra: std::collections::BTreeMap<String, Value>,
    },
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserManager {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default, rename = "$ref", alias = "ref")]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScimEnterpriseUser {
    #[serde(default, rename = "employeeNumber")]
    pub employee_number: Option<String>,
    #[serde(default, rename = "costCenter")]
    pub cost_center: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub division: Option<String>,
    #[serde(default)]
    pub department: Option<String>,
    #[serde(default)]
    pub manager: Option<UserManager>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScimMember {
    pub value: String,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default, deserialize_with = "string_or_none")]
    pub kind: Option<String>,
}
fn string_or_none<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer).unwrap_or(None))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub schemas: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "externalId")]
    pub external_id: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub members: Option<Vec<ScimMember>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    #[serde(default = "patch_schema")]
    pub schemas: Vec<String>,
    #[serde(rename = "Operations")]
    pub operations: Vec<PatchOperation>,
}
fn patch_schema() -> Vec<String> {
    vec![PATCH_SCHEMA.to_owned()]
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PatchOperation {
    pub op: String,
    pub path: Option<String>,
    pub value: Option<Value>,
}
impl<'de> Deserialize<'de> for PatchOperation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Input {
            op: String,
            #[serde(default)]
            path: Option<String>,
            #[serde(default)]
            value: Option<Value>,
        }
        let input = Input::deserialize(deserializer)?;
        let op = input.op.to_lowercase();
        if !matches!(op.as_str(), "add" | "remove" | "replace") {
            return Err(serde::de::Error::custom(
                "op must be add, remove, or replace",
            ));
        }
        Ok(Self {
            op,
            path: input.path,
            value: input.value,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScimSchemaExtension {
    #[serde(rename = "schema", alias = "schema_")]
    pub schema_: String,
    pub required: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScimResourceType {
    #[serde(default = "resource_type_schema")]
    pub schemas: Vec<String>,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub endpoint: String,
    #[serde(rename = "schema", alias = "schema_")]
    pub schema_: String,
    #[serde(default, rename = "schemaExtensions")]
    pub schema_extensions: Option<Vec<ScimSchemaExtension>>,
    #[serde(default)]
    pub meta: Option<Value>,
}
fn resource_type_schema() -> Vec<String> {
    vec!["urn:ietf:params:scim:schemas:core:2.0:ResourceType".to_owned()]
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScimSchemaAttribute {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, rename = "multiValued")]
    pub multi_valued: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default = "default_read_write")]
    pub mutability: String,
    #[serde(default = "default_returned")]
    pub returned: String,
    #[serde(default = "default_unique")]
    pub uniqueness: String,
    #[serde(default, rename = "subAttributes")]
    pub sub_attributes: Option<Vec<ScimSchemaAttribute>>,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScimSchema {
    #[serde(default = "schema_schema")]
    pub schemas: Vec<String>,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attributes: Vec<ScimSchemaAttribute>,
    #[serde(default)]
    pub meta: Option<Value>,
}
fn schema_schema() -> Vec<String> {
    vec!["urn:ietf:params:scim:schemas:core:2.0:Schema".to_owned()]
}
