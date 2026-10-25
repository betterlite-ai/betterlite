use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub(crate) const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    User,
    Group,
    Patch,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DecodedScimResource {
    User(Box<User>),
    Group(Box<Group>),
    Patch(Box<Patch>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScimValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "UserInput")]
pub struct User {
    pub schemas: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "externalId")]
    pub external_id: Option<String>,
    #[serde(default)]
    pub meta: Option<ScimValue>,
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
#[derive(Deserialize)]
struct UserInput {
    schemas: Vec<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "externalId")]
    external_id: Option<String>,
    #[serde(default)]
    meta: Option<ScimValue>,
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
impl From<UserInput> for User {
    fn from(input: UserInput) -> Self {
        Self {
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
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEmail {
    pub value: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub primary: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserGroup {
    pub value: String,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default = "default_direct", rename = "type")]
    pub kind: String,
}
fn default_direct() -> String {
    let mut value = String::new();
    value.push_str("direct");
    value
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, from = "ScimMultiValuedAttributeInput")]
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
        extra: BTreeMap<String, ScimValue>,
    },
    String(String),
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum ScimMultiValuedAttributeInput {
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
        extra: BTreeMap<String, ScimValue>,
    },
    String(String),
}
impl From<ScimMultiValuedAttributeInput> for ScimMultiValuedAttribute {
    fn from(input: ScimMultiValuedAttributeInput) -> Self {
        match input {
            ScimMultiValuedAttributeInput::Object {
                value,
                display,
                kind,
                primary,
                extra,
            } => Self::Object {
                value,
                display,
                kind,
                primary,
                extra,
            },
            ScimMultiValuedAttributeInput::String(value) => Self::Object {
                value: Some(value),
                display: None,
                kind: None,
                primary: None,
                extra: BTreeMap::new(),
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserManager {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default, rename = "$ref", alias = "ref")]
    pub reference: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ScimMemberInput")]
#[serde(rename_all = "camelCase")]
pub struct ScimMember {
    pub value: String,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}
#[derive(Deserialize)]
struct ScimMemberInput {
    value: String,
    #[serde(default)]
    display: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<ScimValue>,
}
impl From<ScimMemberInput> for ScimMember {
    fn from(input: ScimMemberInput) -> Self {
        Self {
            value: input.value,
            display: input.display,
            kind: match input.kind {
                Some(ScimValue::String(value)) => Some(value),
                _ => None,
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub schemas: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "externalId")]
    pub external_id: Option<String>,
    #[serde(default)]
    pub meta: Option<ScimValue>,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub members: Option<Vec<ScimMember>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    #[serde(default = "patch_schema")]
    pub schemas: Vec<String>,
    #[serde(rename = "Operations")]
    pub operations: Vec<PatchOperation>,
}
fn patch_schema() -> Vec<String> {
    let schema = PATCH_SCHEMA.to_owned();
    vec![schema]
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchOperation {
    pub op: String,
    pub path: Option<String>,
    #[serde(default)]
    pub value: Option<ScimValue>,
}
