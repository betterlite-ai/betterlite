use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

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
