use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use thiserror::Error;

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
