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
    pub id: Option<String>,
    #[serde(default, rename = "externalId")]
    pub external_id: Option<String>,
    pub meta: Option<ScimValue>,
    #[serde(default, rename = "userName")]
    pub user_name: Option<String>,
    pub name: Option<UserName>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    pub active: bool,
    #[serde(skip)]
    pub active_present: bool,
    pub emails: Option<Vec<UserEmail>>,
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
    id: Option<String>,
    #[serde(default, rename = "externalId")]
    external_id: Option<String>,
    meta: Option<ScimValue>,
    #[serde(default, rename = "userName")]
    user_name: Option<String>,
    name: Option<UserName>,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    active: Option<bool>,
    emails: Option<Vec<UserEmail>>,
    groups: Option<Vec<UserGroup>>,
    entitlements: Option<Vec<ScimMultiValuedAttribute>>,
    roles: Option<Vec<ScimMultiValuedAttribute>>,
    #[serde(
        default,
        rename = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
    )]
    enterprise_user: Option<ScimEnterpriseUser>,
}
impl From<UserInput> for User {
    fn from(input: UserInput) -> Self {
        let active_present = input.active.is_some();
        let active = input.active.unwrap_or(true);
        let schemas = match input.schemas.len() {
            0 => Vec::new(),
            _ => input.schemas,
        };
        Self {
            schemas,
            id: input.id,
            external_id: input.external_id,
            meta: input.meta,
            user_name: input.user_name,
            name: input.name,
            display_name: input.display_name,
            active_present,
            active,
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
    pub family_name: Option<String>,
    pub given_name: Option<String>,
    pub formatted: Option<String>,
    pub middle_name: Option<String>,
    pub honorific_prefix: Option<String>,
    pub honorific_suffix: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEmail {
    pub value: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    pub primary: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserGroup {
    pub value: String,
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
        value: Option<String>,
        display: Option<String>,
        #[serde(default, rename = "type")]
        kind: Option<String>,
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
        value: Option<String>,
        display: Option<String>,
        #[serde(default, rename = "type")]
        kind: Option<String>,
        primary: Option<bool>,
        #[serde(flatten)]
        extra: BTreeMap<String, ScimValue>,
    },
    String(String),
}
impl From<ScimMultiValuedAttributeInput> for ScimMultiValuedAttribute {
    fn from(input: ScimMultiValuedAttributeInput) -> Self {
        let default_extra = BTreeMap::new();
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
                extra: default_extra,
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserManager {
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
    pub organization: Option<String>,
    pub division: Option<String>,
    pub department: Option<String>,
    pub manager: Option<UserManager>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ScimMemberInput")]
#[serde(rename_all = "camelCase")]
pub struct ScimMember {
    pub value: String,
    pub display: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}
#[derive(Deserialize)]
struct ScimMemberInput {
    value: String,
    display: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<ScimValue>,
}
impl From<ScimMemberInput> for ScimMember {
    fn from(input: ScimMemberInput) -> Self {
        let value = input.value;
        let display = input.display;
        let kind = input.kind;
        Self {
            value,
            display,
            kind: match kind {
                Some(ScimValue::String(kind_value)) => Some(kind_value),
                _ => None,
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub schemas: Vec<String>,
    pub id: Option<String>,
    #[serde(default, rename = "externalId")]
    pub external_id: Option<String>,
    pub meta: Option<ScimValue>,
    #[serde(rename = "displayName")]
    pub display_name: String,
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
    pub value: Option<ScimValue>,
}
