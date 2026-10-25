use serde::{Deserialize, Serialize};

use super::attribute::DirectoryAttribute;
use super::email::Email;
use super::lax_bool::ScimBool;
use super::raw_object::RawObject;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub schemas: Vec<String>,
    pub id: Option<String>,
    pub external_id: Option<String>,
    pub meta: Option<RawObject>,
    pub user_name: Option<String>,
    pub name: Option<UserName>,
    pub display_name: Option<String>,
    #[serde(default)]
    pub active: Active,
    pub emails: Option<Vec<UserEmail>>,
    pub groups: Option<Vec<UserGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entitlements: Option<Vec<DirectoryAttribute>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<DirectoryAttribute>>,
    #[serde(
        rename = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User",
        alias = "enterprise_user",
        skip_serializing_if = "Option::is_none"
    )]
    pub enterprise: Option<EnterpriseUser>,
}
// a PUT that omits active must not reactivate a deactivated account, so the decoder keeps whether it was sent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Active {
    Sent(ScimBool),
    #[serde(skip_deserializing)]
    Defaulted(ScimBool),
}
impl Default for Active {
    fn default() -> Self {
        Self::Defaulted(ScimBool(true))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserName {
    pub family_name: Option<String>,
    pub given_name: Option<String>,
    pub formatted: Option<String>,
    pub middle_name: Option<String>,
    pub honorific_prefix: Option<String>,
    pub honorific_suffix: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserEmail {
    pub value: Email,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub primary: Option<ScimBool>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserGroup {
    pub value: String,
    pub display: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: GroupKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupKind(pub Option<String>);
impl Default for GroupKind {
    fn default() -> Self {
        Self(Some(String::from("direct")))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseUser {
    pub employee_number: Option<String>,
    pub cost_center: Option<String>,
    pub organization: Option<String>,
    pub division: Option<String>,
    pub department: Option<String>,
    pub manager: Option<Manager>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manager {
    pub value: Option<String>,
    pub display_name: Option<String>,
    #[serde(rename = "$ref", alias = "ref")]
    pub reference: Option<String>,
}
