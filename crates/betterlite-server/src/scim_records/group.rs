use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use thiserror::Error;

use super::raw_object::RawObject;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub schemas: Vec<String>,
    pub id: Option<String>,
    pub external_id: Option<String>,
    pub meta: Option<RawObject>,
    pub display_name: String,
    pub members: Option<Vec<Member>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub value: String,
    pub display: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: MemberKind,
}
// LiteLLM keeps a member whose type is not a string and drops only the type
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "MemberKindInput")]
pub struct MemberKind(pub Option<String>);
#[derive(Deserialize)]
#[serde(untagged)]
enum MemberKindInput {
    Text(String),
    Other(IgnoredAny),
}
impl From<MemberKindInput> for MemberKind {
    fn from(input: MemberKindInput) -> Self {
        match input {
            MemberKindInput::Text(kind) => Self(Some(kind)),
            MemberKindInput::Other(_) => Self(None),
        }
    }
}
schema_list! { PatchSchemas, "urn:ietf:params:scim:api:messages:2.0:PatchOp" }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    #[serde(default)]
    pub schemas: PatchSchemas,
    #[serde(rename = "Operations")]
    pub operations: Vec<PatchOperation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchOperation {
    pub op: PatchOp,
    pub path: Option<String>,
    pub value: Option<Box<RawValue>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", try_from = "String")]
pub enum PatchOp {
    Add,
    Remove,
    Replace,
}
#[derive(Debug, Error)]
#[error("op must be add, remove, or replace, not {op:?}")]
pub struct PatchOpError {
    pub op: String,
}
impl TryFrom<String> for PatchOp {
    type Error = PatchOpError;
    fn try_from(op: String) -> Result<Self, PatchOpError> {
        match op.to_lowercase().as_str() {
            "add" => Ok(Self::Add),
            "remove" => Ok(Self::Remove),
            "replace" => Ok(Self::Replace),
            _ => Err(PatchOpError { op }),
        }
    }
}
