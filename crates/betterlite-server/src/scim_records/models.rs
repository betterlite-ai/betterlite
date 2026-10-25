use serde::de::{Error as DeError, Visitor};
use serde::ser::SerializeMap as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt::{self, Formatter};

pub(super) const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    User,
    Group,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
pub struct ScimResource {
    pub schemas: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub meta: Option<ScimValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

impl<'de> Deserialize<'de> for User {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = UserInput::deserialize(deserializer)?;
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
    pub kind: Option<String>,
}
fn default_direct() -> Option<String> {
    ["direct"].first().map(|value| (*value).to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
        extra: BTreeMap<String, ScimValue>,
    },
    String(String),
}

impl Serialize for ScimMultiValuedAttribute {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Object {
                value,
                display,
                kind,
                primary,
                extra,
            } => {
                let mut state = serializer.serialize_map(Some(extra.len().saturating_add(4)))?;
                state.serialize_entry("value", value)?;
                state.serialize_entry("display", display)?;
                state.serialize_entry("type", kind)?;
                state.serialize_entry("primary", primary)?;
                for (key, extra_value) in extra {
                    state.serialize_entry(key, extra_value)?;
                }
                state.end()
            }
            Self::String(value) => {
                let mut state = serializer.serialize_map(Some(4))?;
                state.serialize_entry("value", value)?;
                state.serialize_entry("display", &Option::<String>::None)?;
                state.serialize_entry("type", &Option::<String>::None)?;
                state.serialize_entry("primary", &Option::<bool>::None)?;
                state.end()
            }
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
#[serde(rename_all = "camelCase")]
pub struct ScimMember {
    pub value: String,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "string_or_none")]
    pub kind: Option<String>,
}
fn string_or_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_option(StringOrNoneVisitor)
}

struct StringOrNoneVisitor;

impl<'de> Visitor<'de> for StringOrNoneVisitor {
    type Value = Option<String>;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string or null")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(None)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(String::deserialize(deserializer).ok())
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
    vec![PATCH_SCHEMA.to_owned()]
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PatchOperation {
    pub op: String,
    pub path: Option<String>,
    pub value: Option<ScimValue>,
}
impl<'de> Deserialize<'de> for PatchOperation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Input {
            op: String,
            #[serde(default)]
            path: Option<String>,
            #[serde(default)]
            value: Option<ScimValue>,
        }
        let input = Input::deserialize(deserializer)?;
        let op = input.op.to_lowercase();
        if !matches!(op.as_str(), "add" | "remove" | "replace") {
            return Err(DeError::custom("op must be add, remove, or replace"));
        }
        Ok(Self {
            op,
            path: input.path,
            value: input.value,
        })
    }
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
    #[serde(default)]
    pub description: Option<String>,
    pub endpoint: String,
    #[serde(rename = "schema", alias = "schema_")]
    pub schema_: String,
    #[serde(default, rename = "schemaExtensions")]
    pub schema_extensions: Option<Vec<ScimSchemaExtension>>,
    #[serde(default)]
    pub meta: Option<ScimValue>,
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
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attributes: Vec<ScimSchemaAttribute>,
    #[serde(default)]
    pub meta: Option<ScimValue>,
}
fn schema_schema() -> Vec<String> {
    vec!["urn:ietf:params:scim:schemas:core:2.0:Schema".to_owned()]
}
