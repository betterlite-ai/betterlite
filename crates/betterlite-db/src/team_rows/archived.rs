use super::{
    decoder::{TeamRowError, convert},
    team::LiteLLMTeamTable,
};
use crate::budget_permission_rows::{JsonData, Timestamp};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct LiteLLMTeamTableCachedObj {
    #[serde(flatten)]
    pub team: LiteLLMTeamTable,
    pub last_refreshed_at: Option<f64>,
}
#[derive(Deserialize)]
struct CacheFields {
    last_refreshed_at: Option<f64>,
}
impl TryFrom<JsonData> for LiteLLMTeamTableCachedObj {
    type Error = TeamRowError;
    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        with_team::<CacheFields>(&value).map(|(team, cache)| Self {
            team,
            last_refreshed_at: cache.last_refreshed_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct LiteLLMDeletedTeamTable {
    #[serde(flatten)]
    pub team: LiteLLMTeamTable,
    #[serde(flatten)]
    pub audit: Deletion,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deletion {
    pub id: Option<String>,
    pub deleted_at: Option<Timestamp>,
    pub deleted_by: Option<String>,
    pub deleted_by_api_key: Option<String>,
    pub litellm_changed_by: Option<String>,
}
impl TryFrom<JsonData> for LiteLLMDeletedTeamTable {
    type Error = TeamRowError;
    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        let (team, audit) = with_team(&value)?;
        Ok(Self { team, audit })
    }
}
fn with_team<T>(value: &JsonData) -> Result<(LiteLLMTeamTable, T), TeamRowError>
where
    T: DeserializeOwned,
{
    let team = LiteLLMTeamTable::try_from(value.clone())?;
    let extension = convert(value)?;
    Ok((team, extension))
}
