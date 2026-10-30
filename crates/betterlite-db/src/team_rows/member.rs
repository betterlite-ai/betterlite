use super::decoder::TeamRowError;
use crate::budget_permission_rows::{JsonData, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct MemberBase {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct Member {
    #[serde(flatten)]
    pub base: MemberBase,
    pub role: MemberRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberRole {
    Admin,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLimitEntry {
    pub budget_duration: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub max_budget: Decimal,
    #[serde(default)]
    pub reset_at: Option<Timestamp>,
}

impl TryFrom<JsonData> for MemberBase {
    type Error = TeamRowError;
    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        let JsonData::Object(fields) = &value else {
            return Err(TeamRowError::NotAnObject);
        };
        if ["user_id", "user_email"]
            .iter()
            .all(|key| matches!(fields.get(*key), None | Some(JsonData::Null)))
        {
            return Err(TeamRowError::MissingIdentity);
        }
        let identity: Identity = super::decoder::convert(&value)?;
        Ok(Self {
            user_id: identity.user_id,
            user_email: identity.user_email,
        })
    }
}
#[derive(Deserialize)]
struct Identity {
    user_id: Option<String>,
    user_email: Option<String>,
}

#[derive(Deserialize)]
struct MemberInput {
    role: MemberRole,
}
impl TryFrom<JsonData> for Member {
    type Error = TeamRowError;
    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        let base = MemberBase::try_from(value.clone())?;
        let input: MemberInput = super::decoder::convert(&value)?;
        Ok(Self {
            base,
            role: input.role,
        })
    }
}
